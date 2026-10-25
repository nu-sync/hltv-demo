//! URL grammar and page identity. Failures are values: a URL that is not hltv.org, or not a page this
//! tool reads, comes back as an `HltvError`, so every boundary can reject it before a browser opens.

use crate::core::errors::HltvError;

pub const BASE: &str = "https://www.hltv.org";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Match { match_id: u64, url: String },
    Results { url: String },
    Demo { demo_id: u64, url: String },
}

impl Target {
    pub fn url(&self) -> &str {
        match self {
            Self::Match { url, .. } | Self::Results { url } | Self::Demo { url, .. } => url,
        }
    }
}

/// Path and query of an hltv.org URL, or of a bare path.
pub fn path_of(url: &str) -> Result<String, HltvError> {
    if url.starts_with('/') {
        return Ok(url.to_string());
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let (host, path) = rest
        .find('/')
        .map_or((rest, "/"), |slash| rest.split_at(slash));
    match host.to_ascii_lowercase().as_str() {
        "hltv.org" | "www.hltv.org" => Ok(path.to_string()),
        _ => Err(HltvError::NotHltvUrl {
            url: url.to_string(),
        }),
    }
}

/// Absolute hltv.org URL, so a bare path and a full URL behave the same.
pub fn absolute(url: &str) -> Result<String, HltvError> {
    Ok(format!("{BASE}{}", path_of(url)?))
}

/// Identifies the page a URL opens: a match or demo by id (the slug is not identity), else path+query.
/// Neither a fragment nor Cloudflare's `__cf_chl_*` parameters, which its challenge adds to the URL it
/// is solving, change which page it is.
pub fn page_key(url: &str) -> String {
    let Ok(path) = path_of(url) else {
        return url.to_string();
    };
    let path = path
        .split_once('#')
        .map_or(path.as_str(), |(before, _)| before);
    let path = match path.split_once('?') {
        None => path.to_string(),
        Some((base, query)) => {
            let kept: Vec<&str> = query
                .split('&')
                .filter(|p| !p.starts_with("__cf_chl_"))
                .collect();
            if kept.is_empty() {
                base.to_string()
            } else {
                format!("{base}?{}", kept.join("&"))
            }
        }
    };
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        ["matches", id, ..] => format!("/matches/{id}"),
        ["download", "demo", id, ..] => format!("/download/demo/{id}"),
        _ => path,
    }
}

/// Read a URL as one of the three shapes this tool understands.
pub fn classify(url: &str) -> Result<Target, HltvError> {
    let path = path_of(url)?;
    let key = page_key(url);
    let id = |prefix: &str| {
        key.strip_prefix(prefix)
            .and_then(|id| id.parse::<u64>().ok())
    };
    if let Some(match_id) = id("/matches/") {
        return Ok(Target::Match {
            match_id,
            url: format!("{BASE}{path}"),
        });
    }
    if let Some(demo_id) = id("/download/demo/") {
        return Ok(Target::Demo {
            demo_id,
            url: format!("{BASE}{key}"),
        });
    }
    if path == "/results" || path.starts_with("/results?") {
        return Ok(Target::Results {
            url: format!("{BASE}{path}"),
        });
    }
    Err(HltvError::UnsupportedPage {
        url: url.to_string(),
    })
}

/// hltv.org map filter with the `de_` prefix HLTV expects.
pub fn map_filter(map: &str) -> Result<String, HltvError> {
    let name = map.trim().to_ascii_lowercase();
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(HltvError::NotAMap {
            map: map.to_string(),
        });
    }
    Ok(if name.starts_with("de_") {
        name
    } else {
        format!("de_{name}")
    })
}

/// The filter a results URL already carries, so a URL is never silently widened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultsFilter {
    pub team: Option<u64>,
    pub map: Option<String>,
    pub offset: Option<u64>,
}

pub fn results_filter(url: &str) -> Result<ResultsFilter, HltvError> {
    let path = path_of(url)?;
    let query = path.split_once('?').map_or("", |(_, q)| q);
    let mut filter = ResultsFilter::default();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "team" => filter.team = value.parse().ok().filter(|&t| t > 0),
            "map" => filter.map = Some(value.into_owned()),
            "offset" => filter.offset = value.parse().ok().filter(|&o| o > 0),
            _ => {}
        }
    }
    Ok(filter)
}

/// A results URL for a team and/or map, paged by offset.
pub fn results_url(
    team: Option<u64>,
    map: Option<&str>,
    offset: Option<u64>,
) -> Result<String, HltvError> {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if let Some(team) = team {
        query.append_pair("team", &team.to_string());
    }
    if let Some(map) = map {
        query.append_pair("map", &map_filter(map)?);
    }
    if let Some(offset) = offset.filter(|&o| o > 0) {
        query.append_pair("offset", &offset.to_string());
    }
    let query = query.finish();
    Ok(if query.is_empty() {
        format!("{BASE}/results")
    } else {
        format!("{BASE}/results?{query}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_of_accepts_hltv_org_www_bare_paths_and_http() {
        assert_eq!(
            path_of("https://www.hltv.org/results?team=1").unwrap(),
            "/results?team=1"
        );
        assert_eq!(
            path_of("https://hltv.org/matches/1/x").unwrap(),
            "/matches/1/x"
        );
        assert_eq!(path_of("http://hltv.org/results").unwrap(), "/results");
        assert_eq!(path_of("/download/demo/9").unwrap(), "/download/demo/9");
    }

    #[test]
    fn path_of_refuses_every_other_host_including_look_alikes() {
        for url in [
            "https://example.com/results",
            "https://hltv.org.evil.com/results",
            "https://not-hltv.org/x",
        ] {
            assert_eq!(
                path_of(url),
                Err(HltvError::NotHltvUrl { url: url.into() }),
                "{url}"
            );
        }
    }

    #[test]
    fn page_key_identifies_by_match_and_demo_id_ignoring_the_slug() {
        assert_eq!(
            page_key("https://www.hltv.org/matches/2399723/vitality-vs-9z"),
            "/matches/2399723"
        );
        assert_eq!(page_key("/matches/2399723/other-slug"), "/matches/2399723");
        assert_eq!(
            page_key("https://www.hltv.org/download/demo/113159"),
            "/download/demo/113159"
        );
    }

    #[test]
    fn page_key_keeps_path_and_query_for_everything_else() {
        assert_ne!(
            page_key("https://www.hltv.org/results?team=9565&map=de_cache"),
            page_key("https://www.hltv.org/results?team=9565&map=de_inferno")
        );
        assert_eq!(
            page_key("https://www.hltv.org/results?offset=100"),
            "/results?offset=100"
        );
    }

    #[test]
    fn page_key_ignores_cloudflares_challenge_parameters_and_the_fragment() {
        assert_eq!(
            page_key("https://www.hltv.org/results?team=9565&__cf_chl_rt_tk=abc-1.0"),
            "/results?team=9565"
        );
        assert_eq!(
            page_key("https://www.hltv.org/results?__cf_chl_tk=abc"),
            "/results"
        );
        assert_eq!(
            page_key("https://www.hltv.org/results?team=1#top"),
            "/results?team=1"
        );
    }

    #[test]
    fn classify_recognizes_the_three_shapes() {
        assert!(matches!(
            classify("https://www.hltv.org/matches/2399725/g2-vs-navi").unwrap(),
            Target::Match {
                match_id: 2399725,
                ..
            }
        ));
        assert_eq!(
            classify("/results?team=9565").unwrap(),
            Target::Results {
                url: format!("{BASE}/results?team=9565")
            }
        );
        assert_eq!(
            classify("/download/demo/113166").unwrap(),
            Target::Demo {
                demo_id: 113166,
                url: format!("{BASE}/download/demo/113166")
            }
        );
    }

    #[test]
    fn classify_refuses_other_hltv_pages() {
        for url in [
            "https://www.hltv.org/player/7998/s1mple",
            "https://www.hltv.org/team/9565/vitality",
            "https://www.hltv.org/matches/abc",
        ] {
            assert_eq!(
                classify(url),
                Err(HltvError::UnsupportedPage { url: url.into() }),
                "{url}"
            );
        }
    }

    #[test]
    fn the_map_filter_gets_the_de_prefix_once() {
        assert_eq!(map_filter("cache").unwrap(), "de_cache");
        assert_eq!(map_filter("de_cache").unwrap(), "de_cache");
        assert_eq!(map_filter("Anubis").unwrap(), "de_anubis");
        assert_eq!(
            map_filter("cache&x=1"),
            Err(HltvError::NotAMap {
                map: "cache&x=1".into()
            })
        );
        assert!(map_filter("  ").is_err());
    }

    #[test]
    fn results_url_encodes_only_what_it_was_given() {
        assert_eq!(
            results_url(Some(9565), Some("cache"), None).unwrap(),
            "https://www.hltv.org/results?team=9565&map=de_cache"
        );
        assert_eq!(
            results_url(None, None, None).unwrap(),
            "https://www.hltv.org/results"
        );
        assert_eq!(
            results_url(None, None, Some(100)).unwrap(),
            "https://www.hltv.org/results?offset=100"
        );
        assert_eq!(
            results_url(None, None, Some(0)).unwrap(),
            "https://www.hltv.org/results"
        );
    }

    #[test]
    fn absolute_resolves_bare_paths_and_the_bare_host() {
        assert_eq!(
            absolute("/download/demo/1").unwrap(),
            "https://www.hltv.org/download/demo/1"
        );
        assert_eq!(
            absolute("https://hltv.org/results").unwrap(),
            "https://www.hltv.org/results"
        );
    }

    #[test]
    fn results_filter_keeps_the_filter_a_url_already_carries() {
        assert_eq!(
            results_filter("https://www.hltv.org/results?team=9565&map=de_inferno").unwrap(),
            ResultsFilter {
                team: Some(9565),
                map: Some("de_inferno".into()),
                offset: None
            }
        );
        assert_eq!(
            results_filter("/results?offset=100").unwrap(),
            ResultsFilter {
                team: None,
                map: None,
                offset: Some(100)
            }
        );
        assert_eq!(
            results_filter("https://www.hltv.org/results").unwrap(),
            ResultsFilter::default()
        );
        let kept =
            results_filter("https://www.hltv.org/results?team=9565&map=de_inferno&offset=100")
                .unwrap();
        assert_eq!(
            results_url(kept.team, kept.map.as_deref(), kept.offset).unwrap(),
            format!("{BASE}/results?team=9565&map=de_inferno&offset=100")
        );
    }
}

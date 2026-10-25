use std::collections::HashMap;

use crate::core::dates::from_heading;
use crate::core::errors::HltvError;
use crate::core::types::{Match, SearchFilter};
use crate::core::url::{absolute, results_url};
use crate::hltv::extract::RawMatch;
use crate::source::Source;

/// The most matches one search returns: one hltv.org results page. Page with `offset` for more.
pub const MAX_LIMIT: usize = 100;

/// One results row as a Match, or None when the row's link carries no match id.
pub fn to_match(raw: RawMatch) -> Option<Match> {
    let match_id = raw
        .href
        .strip_prefix("/matches/")?
        .split('/')
        .next()?
        .parse()
        .ok()?;
    Some(Match {
        match_id,
        url: absolute(&raw.href).ok()?,
        date: from_heading(raw.heading.as_deref()),
        event: raw.event,
        teams: raw.teams,
        scores: raw.scores,
    })
}

/// hltv.org shows a match twice: once in the "featured" block, again under its date heading. Keep
/// the first position but prefer the copy that carries a date, so a limit does not spend a slot on a
/// duplicate.
pub fn dedupe(matches: Vec<Match>) -> Vec<Match> {
    let mut kept: Vec<Match> = Vec::new();
    let mut index: HashMap<u64, usize> = HashMap::new();
    for current in matches {
        match index.get(&current.match_id) {
            None => {
                index.insert(current.match_id, kept.len());
                kept.push(current);
            }
            Some(&at) if kept[at].date.is_none() && current.date.is_some() => {
                let seen = &mut kept[at];
                seen.date = current.date;
                seen.event = seen.event.take().or(current.event);
            }
            Some(_) => {}
        }
    }
    kept
}

/// The newest matches on a results page: `limit` of them, default 1 and at most MAX_LIMIT.
pub async fn search(source: &impl Source, filter: &SearchFilter) -> Result<Vec<Match>, HltvError> {
    let url = results_url(filter.team, filter.map.as_deref(), filter.offset)?;
    let raw = source.results(&url).await?;
    let mut matches = dedupe(raw.into_iter().filter_map(to_match).collect());
    if matches.is_empty() {
        return Err(HltvError::NoMatchesFound { url });
    }
    matches.truncate(filter.limit.unwrap_or(1).clamp(1, MAX_LIMIT));
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(match_id: u64, date: Option<&str>) -> Match {
        Match {
            match_id,
            url: format!("https://www.hltv.org/matches/{match_id}/x"),
            date: date.map(String::from),
            event: Some("E".into()),
            teams: ("A".into(), "B".into()),
            scores: Some((2, 0)),
        }
    }

    #[test]
    fn to_match_reads_the_id_from_the_link_not_the_slug() {
        let found = to_match(RawMatch {
            href: "/matches/2399723/vitality-vs-9z-anything".into(),
            teams: ("Vitality".into(), "9z".into()),
            scores: Some((2, 0)),
            event: Some("PGL Masters Bucharest 2026".into()),
            heading: Some("Results for October 24th 2026".into()),
        })
        .unwrap();
        assert_eq!(found.match_id, 2399723);
        assert_eq!(
            found.url,
            "https://www.hltv.org/matches/2399723/vitality-vs-9z-anything"
        );
        assert_eq!(found.date.as_deref(), Some("2026-10-24"));
        assert_eq!(found.teams, ("Vitality".into(), "9z".into()));
    }

    #[test]
    fn to_match_drops_a_row_without_a_match_id() {
        let row = RawMatch {
            href: "/matches/live".into(),
            teams: Default::default(),
            scores: None,
            event: None,
            heading: None,
        };
        assert_eq!(to_match(row), None);
    }

    #[test]
    fn dedupe_keeps_the_position_but_takes_the_dated_copy() {
        let kept = dedupe(vec![
            found(1, None),
            found(1, Some("2026-10-24")),
            found(2, Some("2026-10-11")),
        ]);
        assert_eq!(
            kept.iter().map(|m| m.match_id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(kept[0].date.as_deref(), Some("2026-10-24"));
    }

    #[test]
    fn dedupe_leaves_distinct_matches_alone() {
        assert_eq!(
            dedupe(vec![
                found(1, Some("2026-10-24")),
                found(2, Some("2026-10-11"))
            ])
            .len(),
            2
        );
    }
}

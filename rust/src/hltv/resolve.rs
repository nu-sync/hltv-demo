use crate::core::errors::HltvError;
use crate::core::types::DemoRef;
use crate::core::url::{Target, absolute, classify};
use crate::source::Source;

/// The demo archive URL of a match page, or NoDemoFound when the match publishes none.
pub async fn resolve(source: &impl Source, match_url: &str) -> Result<DemoRef, HltvError> {
    let no_demo = || HltvError::NoDemoFound {
        url: match_url.to_string(),
    };
    let Target::Match { match_id, url } = classify(match_url)? else {
        return Err(no_demo());
    };
    let page = source.match_page(&url).await?;
    let href = page.demo_href.ok_or_else(no_demo)?;
    let demo_id = href
        .split("/download/demo/")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|id| id.parse().ok())
        .ok_or_else(no_demo)?;
    Ok(DemoRef {
        match_id,
        demo_id,
        url: absolute(&href)?,
        match_url: url,
    })
}

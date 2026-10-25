const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// ISO day from a results-list heading such as "Results for October 24th 2026".
/// Returns None for anything else, so an unknown heading costs a date field, not a whole search.
pub fn from_heading(heading: Option<&str>) -> Option<String> {
    let words: Vec<&str> = heading?.split_whitespace().collect();
    words.windows(3).find_map(|w| {
        let month = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(w[0]))? + 1;
        let digits = ["st", "nd", "rd", "th"]
            .iter()
            .find_map(|s| w[1].strip_suffix(s))
            .unwrap_or(w[1]);
        let day: u32 = digits
            .parse()
            .ok()
            .filter(|d| (1..=31).contains(d) && digits.len() <= 2)?;
        let year: u32 = w[2].parse().ok().filter(|_| w[2].len() == 4)?;
        Some(format!("{year}-{month:02}-{day:02}"))
    })
}

#[cfg(test)]
mod tests {
    use super::from_heading;

    #[test]
    fn parses_hltv_results_headings() {
        assert_eq!(
            from_heading(Some("Results for October 24th 2026")).as_deref(),
            Some("2026-10-24")
        );
        assert_eq!(
            from_heading(Some("Results for January 1st 2027")).as_deref(),
            Some("2027-01-01")
        );
        assert_eq!(
            from_heading(Some("Results for March 22nd 2026")).as_deref(),
            Some("2026-03-22")
        );
    }

    #[test]
    fn returns_none_rather_than_guessing() {
        assert_eq!(from_heading(None), None);
        assert_eq!(from_heading(Some("")), None);
        assert_eq!(from_heading(Some("Featured results")), None);
        assert_eq!(from_heading(Some("Results for Octember 24th 2026")), None);
        assert_eq!(from_heading(Some("Results for October 32nd 2026")), None);
    }
}

//! Byte offsets into UTF-8 text, rounded to a character boundary so they can slice it.

/// The nearest boundary at or below `at`.
pub fn floor(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The nearest boundary at or above `at`.
pub fn ceil(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

#[cfg(test)]
mod tests {
    #[test]
    fn rounds_to_the_edges_of_a_multibyte_character() {
        let text = "a–b"; // the dash is three bytes, at 1..4
        assert_eq!((super::floor(text, 2), super::ceil(text, 2)), (1, 4));
        assert_eq!(
            (super::floor(text, 4), super::ceil(text, 99)),
            (4, text.len())
        );
    }
}

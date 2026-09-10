use unicode_normalization::UnicodeNormalization;

use crate::cleaners::text::CleanResult;

pub(crate) fn clean_tax_id(input: &str, max_len: usize) -> CleanResult {
    if input.chars().count() > max_len {
        return CleanResult::Invalid;
    }
    let nfkc: String = input.nfkc().collect();
    let trimmed = nfkc.trim();
    if trimmed.is_empty() {
        return CleanResult::Empty;
    }
    let upper = trimmed.to_uppercase();
    let mut result = String::new();
    for c in upper.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c);
        } else if c == '-' || c == ' ' || c.is_whitespace() {
            // Discard spaces and -
            continue;
        } else {
            // Reject other characters
            return CleanResult::Invalid;
        }
    }
    if result.is_empty() {
        // Reject empty result
        return CleanResult::Invalid;
    }
    CleanResult::Value(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_tax_id() {
        assert_eq!(
            clean_tax_id("12-3456789", 64),
            CleanResult::Value("123456789".into())
        );
        assert_eq!(
            clean_tax_id("de 123 456 789", 64),
            CleanResult::Value("DE123456789".into())
        );
        assert_eq!(
            clean_tax_id(" US-12345 ", 64),
            CleanResult::Value("US12345".into())
        );
        assert_eq!(clean_tax_id("12.345.678", 64), CleanResult::Invalid);
        assert_eq!(clean_tax_id("--", 64), CleanResult::Invalid);
        assert_eq!(clean_tax_id("   ", 64), CleanResult::Empty);
    }
}

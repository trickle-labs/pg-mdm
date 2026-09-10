use unicode_normalization::UnicodeNormalization;

use crate::cleaners::text::CleanResult;

pub(crate) fn clean_phone(input: &str, max_len: usize) -> CleanResult {
    if input.chars().count() > max_len {
        return CleanResult::Invalid;
    }
    let nfkc: String = input.nfkc().collect();
    let trimmed = nfkc.trim();
    if trimmed.is_empty() {
        return CleanResult::Empty;
    }
    let (has_plus, rest) = if let Some(stripped) = trimmed.strip_prefix('+') {
        (true, stripped)
    } else {
        (false, trimmed)
    };

    let mut digits = String::new();
    for c in rest.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else if matches!(c, ' ' | '-' | '.' | '(' | ')' | '/') || c.is_whitespace() {
            // Discard common visual separators
            continue;
        } else {
            // Any other character (including secondary '+', letters, or unnormalized digits) is rejected
            return CleanResult::Invalid;
        }
    }

    if digits.len() < 7 || digits.len() > 15 {
        return CleanResult::Invalid;
    }

    let normalized = if has_plus {
        format!("+{digits}")
    } else {
        digits
    };
    CleanResult::Value(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_phone() {
        assert_eq!(
            clean_phone("+1 (555) 234-5678", 64),
            CleanResult::Value("+15552345678".into())
        );
        assert_eq!(
            clean_phone("(555) 234-5678", 64),
            CleanResult::Value("5552345678".into())
        );
        assert_eq!(
            clean_phone("555.234.5678", 64),
            CleanResult::Value("5552345678".into())
        );
        assert_eq!(
            clean_phone("555/234-5678", 64),
            CleanResult::Value("5552345678".into())
        );
        assert_eq!(clean_phone("123456", 64), CleanResult::Invalid);
        assert_eq!(clean_phone("+1234567890123456", 64), CleanResult::Invalid);
        assert_eq!(clean_phone("1-800-FLOWERS", 64), CleanResult::Invalid);
        assert_eq!(clean_phone("123+4567890", 64), CleanResult::Invalid);
        assert_eq!(clean_phone("   ", 64), CleanResult::Empty);
        // Fullwidth digits and plus
        assert_eq!(
            clean_phone("＋１　５５５　２３４５６７８", 64),
            CleanResult::Value("+15552345678".into())
        );
    }
}

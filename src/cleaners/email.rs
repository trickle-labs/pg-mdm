use unicode_normalization::UnicodeNormalization;

use crate::cleaners::text::CleanResult;

pub(crate) fn clean_email(input: &str, max_len: usize) -> CleanResult {
    if input.chars().count() > max_len {
        return CleanResult::Invalid;
    }
    let nfkc: String = input.nfkc().collect();
    let trimmed = nfkc.trim();
    if trimmed.is_empty() {
        return CleanResult::Empty;
    }
    if trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return CleanResult::Invalid;
    }
    let parts: Vec<&str> = trimmed.split('@').collect();
    if parts.len() != 2 {
        return CleanResult::Invalid;
    }
    let local = parts[0];
    let domain = parts[1];
    if local.is_empty() || domain.is_empty() {
        return CleanResult::Invalid;
    }
    if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
        return CleanResult::Invalid;
    }
    CleanResult::Value(format!(
        "{}@{}",
        local.to_lowercase(),
        domain.to_lowercase()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_email() {
        assert_eq!(
            clean_email("User.Name+tag@Example.COM", 320),
            CleanResult::Value("user.name+tag@example.com".into())
        );
        assert_eq!(clean_email("   ", 320), CleanResult::Empty);
        assert_eq!(clean_email("user@domain@com", 320), CleanResult::Invalid);
        assert_eq!(clean_email("user.example.com", 320), CleanResult::Invalid);
        assert_eq!(clean_email("@example.com", 320), CleanResult::Invalid);
        assert_eq!(clean_email("user@", 320), CleanResult::Invalid);
        assert_eq!(clean_email("us er@example.com", 320), CleanResult::Invalid);
        assert_eq!(clean_email("user\n@example.com", 320), CleanResult::Invalid);
    }
}

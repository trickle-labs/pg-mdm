use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CleanResult {
    Value(String),
    Empty,
    Invalid,
    Unsupported,
}

pub(crate) fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_whitespace = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !in_whitespace {
                result.push(' ');
                in_whitespace = true;
            }
        } else {
            result.push(c);
            in_whitespace = false;
        }
    }
    result
}

pub(crate) fn clean_text(input: &str, max_len: usize) -> CleanResult {
    if input.chars().count() > max_len {
        return CleanResult::Invalid;
    }
    let nfkc: String = input.nfkc().collect();
    let lower = nfkc.to_lowercase();
    let trimmed = lower.trim();
    if trimmed.is_empty() {
        return CleanResult::Empty;
    }
    CleanResult::Value(collapse_whitespace(trimmed))
}

pub(crate) fn clean_person_name(input: &str, max_len: usize) -> CleanResult {
    match clean_text(input, max_len) {
        CleanResult::Value(text) => {
            let replaced: String = text
                .chars()
                .map(|c| if c.is_ascii_punctuation() { ' ' } else { c })
                .collect();
            let trimmed = replaced.trim();
            if trimmed.is_empty() {
                CleanResult::Empty
            } else {
                CleanResult::Value(collapse_whitespace(trimmed))
            }
        }
        other => other,
    }
}

pub(crate) fn clean_company_name(input: &str, max_len: usize) -> CleanResult {
    clean_person_name(input, max_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_text() {
        assert_eq!(
            clean_text("  Hello,   World!  ", 4096),
            CleanResult::Value("hello, world!".into())
        );
        assert_eq!(clean_text("   \t\n  ", 4096), CleanResult::Empty);
        assert_eq!(clean_text("hello", 3), CleanResult::Invalid);
        assert_eq!(clean_text("ﬁle", 4096), CleanResult::Value("file".into()));
    }

    #[test]
    fn test_clean_person_name() {
        assert_eq!(
            clean_person_name("John-Paul Smith, Jr.", 4096),
            CleanResult::Value("john paul smith jr".into())
        );
        assert_eq!(clean_person_name("---, ..", 4096), CleanResult::Empty);
    }

    #[test]
    fn test_clean_company_name() {
        assert_eq!(
            clean_company_name("Acme Corp., LLC", 4096),
            CleanResult::Value("acme corp llc".into())
        );
    }
}

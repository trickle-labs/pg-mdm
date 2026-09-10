use crate::cleaners::text::CleanResult;

pub(crate) fn clean_date_parts(year: i32, month: u8, day: u8) -> CleanResult {
    // Standard ISO 4-digit positive year range
    if !(1..=9999).contains(&year) {
        return CleanResult::Unsupported;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return CleanResult::Invalid;
    }

    // Days in month check (including leap years)
    let max_days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
            if is_leap { 29 } else { 28 }
        }
        _ => return CleanResult::Invalid,
    };

    if day > max_days {
        return CleanResult::Invalid;
    }

    CleanResult::Value(format!("{year:04}-{month:02}-{day:02}"))
}

pub(crate) fn clean_date_str(input: &str) -> CleanResult {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return CleanResult::Empty;
    }
    let parts: Vec<&str> = trimmed.split('-').collect();
    if parts.len() != 3 {
        return CleanResult::Invalid;
    }
    let Ok(year) = parts[0].parse::<i32>() else {
        return CleanResult::Invalid;
    };
    let Ok(month) = parts[1].parse::<u8>() else {
        return CleanResult::Invalid;
    };
    let Ok(day) = parts[2].parse::<u8>() else {
        return CleanResult::Invalid;
    };
    clean_date_parts(year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_date() {
        assert_eq!(
            clean_date_str("2026-09-10"),
            CleanResult::Value("2026-09-10".into())
        );
        assert_eq!(
            clean_date_str("2024-02-29"),
            CleanResult::Value("2024-02-29".into())
        );
        assert_eq!(clean_date_str("2023-02-29"), CleanResult::Invalid);
        assert_eq!(clean_date_str("0000-01-01"), CleanResult::Unsupported);
        assert_eq!(clean_date_str("-0001-01-01"), CleanResult::Invalid);
        assert_eq!(clean_date_str("10000-01-01"), CleanResult::Unsupported);
        assert_eq!(clean_date_str("invalid-date"), CleanResult::Invalid);
        assert_eq!(clean_date_str("   "), CleanResult::Empty);
    }
}

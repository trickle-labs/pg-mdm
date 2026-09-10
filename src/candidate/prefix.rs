pub fn prefix_key(normalized: &str, length: usize) -> Vec<u8> {
    normalized
        .chars()
        .take(length)
        .collect::<String>()
        .into_bytes()
}

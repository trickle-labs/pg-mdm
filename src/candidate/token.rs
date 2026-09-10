pub fn token_keys(normalized: &str, min_length: usize) -> Vec<Vec<u8>> {
    let mut tokens = normalized
        .split_whitespace()
        .filter(|token| token.chars().count() >= min_length)
        .map(str::as_bytes)
        .map(<[u8]>::to_vec)
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

//! Tiny built-in price table (USD per million tokens). Approximate; `None` when unknown.

/// (model substring, input $/Mtok, output $/Mtok). First match wins, most specific first.
const TABLE: &[(&str, f64, f64)] = &[
    ("claude-opus", 5.0, 25.0),
    ("claude-sonnet", 3.0, 15.0),
    ("claude-haiku", 1.0, 5.0),
    ("gpt-4o-mini", 0.15, 0.60),
    ("gpt-4o", 2.5, 10.0),
    ("gpt-4.1-mini", 0.40, 1.60),
    ("gpt-4.1", 2.0, 8.0),
];

pub fn estimate_cost(model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
    let m = model.to_ascii_lowercase();
    let (_, i, o) = TABLE.iter().find(|(k, _, _)| m.contains(k))?;
    Some((input_tokens as f64 * i + output_tokens as f64 * o) / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown() {
        let c = estimate_cost("claude-sonnet-4-5", 1_000_000, 100_000).unwrap();
        assert!((c - 4.5).abs() < 1e-9);
        assert!(estimate_cost("deepseek-flash", 1, 1).is_none());
        assert!(
            estimate_cost("gpt-4o-mini", 10, 10).unwrap()
                < estimate_cost("gpt-4o", 10, 10).unwrap()
        );
    }
}

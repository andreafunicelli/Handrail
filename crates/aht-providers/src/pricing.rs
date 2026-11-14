//! Tiny built-in price table (USD per million tokens). `None` when the model is not listed:
//! an unknown model shows no cost rather than a wrong one.

/// (model id, input, output, cache read) in $/Mtok. Ids match exactly, optionally followed by
/// a date snapshot suffix. Claude cache writes (5-minute TTL) cost 1.25x input.
/// Claude rows last verified 2026-11-14 against Anthropic's model table (cached 2026-11-03).
/// OpenAI rows are older and unverified; their cached tokens are charged at the input rate.
const TABLE: &[(&str, f64, f64, f64)] = &[
    ("claude-fable-5-2", 8.0, 40.0, 0.20),
    ("claude-mythos-5-2", 8.0, 40.0, 0.20),
    ("claude-fable-5-1", 10.0, 50.0, 0.25),
    ("claude-mythos-5-1", 10.0, 50.0, 0.25),
    ("claude-fable-5", 10.0, 50.0, 1.0),
    ("claude-mythos-5", 10.0, 50.0, 1.0),
    ("claude-opus-5-5", 4.0, 20.0, 0.20),
    ("claude-opus-5", 5.0, 25.0, 0.50),
    ("claude-opus-4-8", 5.0, 25.0, 0.50),
    ("claude-opus-4-7", 5.0, 25.0, 0.50),
    ("claude-opus-4-6", 5.0, 25.0, 0.50),
    ("claude-sonnet-5-5", 2.0, 10.0, 0.10),
    ("claude-sonnet-5", 2.0, 10.0, 0.20),
    ("claude-sonnet-4-6", 3.0, 15.0, 0.30),
    ("claude-haiku-4-5", 1.0, 5.0, 0.10),
    ("gpt-4o-mini", 0.15, 0.60, 0.15),
    ("gpt-4o", 2.5, 10.0, 2.5),
    ("gpt-4.1-mini", 0.40, 1.60, 0.40),
    ("gpt-4.1", 2.0, 8.0, 2.0),
];

/// `model` is exactly `id`, or `id` plus a date snapshot (`-20251001`, `@20251001`,
/// `-2024-08-06`). A provider prefix (`anthropic/`, `anthropic.`) is ignored.
fn matches(model: &str, id: &str) -> bool {
    let m = model.rsplit('/').next().unwrap_or(model);
    let m = m.strip_prefix("anthropic.").unwrap_or(m);
    let Some(rest) = m.strip_prefix(id) else {
        return false;
    };
    let date = |d: &str| {
        let digits: String = d.chars().filter(|c| *c != '-').collect();
        digits.len() == 8 && digits.chars().all(|c| c.is_ascii_digit())
    };
    rest.is_empty()
        || rest
            .strip_prefix(['-', '@'])
            .is_some_and(|d| date(d) && d.len() <= 10)
}

/// Cost of one model call. `input` is every input token, including `cache_read` and
/// `cache_write` ones.
pub fn estimate_cost(
    model: &str,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
) -> Option<f64> {
    let m = model.to_ascii_lowercase();
    let (id, i, o, r) = TABLE.iter().find(|(k, ..)| matches(&m, k))?;
    let write = if id.starts_with("claude-") {
        i * 1.25
    } else {
        *i
    };
    let plain = input.saturating_sub(cache_read + cache_write);
    Some(
        (plain as f64 * i + cache_read as f64 * r + cache_write as f64 * write + output as f64 * o)
            / 1_000_000.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown() {
        let c = estimate_cost("claude-sonnet-4-6", 1_000_000, 100_000, 0, 0).unwrap();
        assert!((c - 4.5).abs() < 1e-9);
        assert!(estimate_cost("deepseek-flash", 1, 1, 0, 0).is_none());
        assert!(
            estimate_cost("gpt-4o-mini", 10, 10, 0, 0).unwrap()
                < estimate_cost("gpt-4o", 10, 10, 0, 0).unwrap()
        );
        // Snapshot suffixes and gateway prefixes still match.
        assert!(estimate_cost("claude-haiku-4-5-20251001", 1, 1, 0, 0).is_some());
        assert!(estimate_cost("anthropic/claude-opus-4-7", 1, 1, 0, 0).is_some());
        assert!(estimate_cost("gpt-4o-2024-08-06", 1, 1, 0, 0).is_some());
    }

    #[test]
    fn unlisted_models_have_no_price_instead_of_a_wrong_one() {
        for m in [
            "claude-opus-4-1",
            "claude-opus-4-20250514",
            "claude-3-5-sonnet-20241022",
            "claude-3-opus-20240229",
            "claude-sonnet-4-5",
            "claude-opus-5-5-fast",
            "claude-haiku-5-5",
        ] {
            assert!(estimate_cost(m, 1, 1, 0, 0).is_none(), "{m}");
        }
        // The closest prefix is not used for another version.
        let a = estimate_cost("claude-opus-5-5", 1_000_000, 0, 0, 0).unwrap();
        let b = estimate_cost("claude-opus-5", 1_000_000, 0, 0, 0).unwrap();
        assert!((a - 4.0).abs() < 1e-9 && (b - 5.0).abs() < 1e-9);
    }

    #[test]
    fn cache_tokens_at_their_own_rates() {
        // 1M input of which 600k cache reads and 200k cache writes.
        let c = estimate_cost("claude-opus-5-5", 1_000_000, 0, 600_000, 200_000).unwrap();
        let want = 0.2 * 4.0 + 0.6 * 0.20 + 0.2 * 5.0;
        assert!((c - want).abs() < 1e-9, "{c} vs {want}");
    }
}

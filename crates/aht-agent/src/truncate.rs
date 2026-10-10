//! Keep tool results fed back to the model small.

use serde_json::Value;

const NOTE: &str =
    "\n...[output shortened for context; use read_output with the output_id to page through the rest]...\n";

fn char_boundary_floor(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn char_boundary_ceil(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(s.len())
}

/// Keep head and tail of `s` so the result is about `budget` bytes, joined by a note.
fn head_tail(s: &str, budget: usize) -> String {
    if s.len() <= budget {
        return s.to_string();
    }
    let keep = budget.saturating_sub(NOTE.len()).max(64);
    let head_end = char_boundary_floor(s, keep * 2 / 3);
    let tail_start = char_boundary_ceil(s, s.len().saturating_sub(keep - keep * 2 / 3));
    format!("{}{}{}", &s[..head_end], NOTE, &s[tail_start..])
}

/// Serialize a tool result for the model, at most about `max_bytes`. If the result is an
/// object with a large string `output` field (run_command / read_output), only that field is
/// shortened (head + tail) so exit codes and ids stay intact; otherwise the JSON text itself
/// is shortened.
pub fn cap_tool_content(value: &Value, max_bytes: usize) -> String {
    let full = value.to_string();
    if full.len() <= max_bytes {
        return full;
    }
    for field in ["output", "data"] {
        if let Some(text) = value.get(field).and_then(|o| o.as_str()) {
            let overhead = full.len() - serde_json::to_string(text).map_or(0, |s| s.len());
            // JSON escaping can inflate the text, so shrink until the whole thing fits.
            let mut budget = max_bytes.saturating_sub(overhead + 64);
            for _ in 0..6 {
                if budget < 256 {
                    break;
                }
                let mut v = value.clone();
                v[field] = Value::String(head_tail(text, budget));
                if v.get("truncated").is_some() {
                    v["truncated"] = Value::Bool(true);
                }
                let s = v.to_string();
                if s.len() <= max_bytes {
                    return s;
                }
                budget = budget * 3 / 4;
            }
        }
    }
    head_tail(&full, max_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn small_values_untouched() {
        let v = json!({"ok": true});
        assert_eq!(cap_tool_content(&v, 1000), v.to_string());
    }

    #[test]
    fn shrinks_output_field_and_keeps_metadata() {
        let big = format!("HEAD{}TAIL", "x".repeat(100_000));
        let v = json!({"state":"completed","exit_code":0,"output":big,"output_id":"abc","truncated":false});
        let s = cap_tool_content(&v, 12 * 1024);
        assert!(s.len() <= 12 * 1024, "{}", s.len());
        let back: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(back["exit_code"], 0);
        assert_eq!(back["output_id"], "abc");
        assert_eq!(back["truncated"], true);
        let o = back["output"].as_str().unwrap();
        assert!(o.starts_with("HEAD") && o.ends_with("TAIL") && o.contains("read_output"));
    }

    #[test]
    fn escape_heavy_and_multibyte_output_still_fits() {
        let big = "é\n\"".repeat(30_000);
        let v = json!({"output": big});
        let s = cap_tool_content(&v, 12 * 1024);
        assert!(s.len() <= 12 * 1024);
        assert!(serde_json::from_str::<Value>(&s).is_ok());
    }

    #[test]
    fn non_object_falls_back_to_text_cut() {
        let v = json!(["a".repeat(50_000)]);
        let s = cap_tool_content(&v, 4096);
        assert!(s.len() <= 4096 + 8);
        assert!(s.contains("read_output"));
    }
}

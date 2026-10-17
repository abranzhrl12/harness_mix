use crate::results::{prefix, utf16_len};
use serde_json::{Value, json};

/// Two adjacent lines per side; bounded in UTF-16 without splitting a character.
pub fn anchors(text: &str, start: usize, end: usize) -> Value {
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<_> = normalized.split('\n').collect();
    let segment = |from: usize, to: usize| {
        let full = lines[from - 1..to].join(if text.contains("\r\n") { "\r\n" } else { "\n" });
        json!({"start":from,"end":to,"text":prefix(&full,1000),"truncated":utf16_len(&full)>1000})
    };
    let before = if start > 1 && start <= lines.len() {
        segment(start.saturating_sub(2).max(1), start - 1)
    } else {
        Value::Null
    };
    let after = if end > 0 && end < lines.len() {
        segment(end + 1, (end + 2).min(lines.len()))
    } else {
        Value::Null
    };
    json!({"before":before,"after":after})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_context_preserves_crlf_and_unicode() {
        let result = anchors("a\r\nb\r\nc\r\nd\r\ne", 3, 3);
        assert_eq!(result["before"]["text"], "a\r\nb");
        assert_eq!(result["after"]["text"], "d\r\ne");
        let text = format!("{}\nx", "🙂".repeat(501));
        assert_eq!(
            anchors(&text, 2, 2)["before"]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            500
        );
        assert_eq!(anchors(&text, 2, 2)["before"]["truncated"], true);
        assert!(anchors("x", 1, 1)["after"].is_null());
    }
}

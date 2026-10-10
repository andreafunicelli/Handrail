//! Minimal Server-Sent Events parser (line based, tolerant of `\r\n` and chunk splits).

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed raw bytes; returns every event completed by them.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if let Some(ev) = self.take() {
                    out.push(ev);
                }
            } else if line.starts_with(':') {
                // comment / keep-alive
            } else {
                let (field, value) = match line.split_once(':') {
                    Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                    None => (line, ""),
                };
                match field {
                    "event" => self.event = Some(value.to_string()),
                    "data" => self.data.push(value.to_string()),
                    _ => {}
                }
            }
        }
        out
    }

    /// Flush a trailing event not terminated by a blank line.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            self.buf.push(b'\n');
            self.push(&[]);
        }
        self.take()
    }

    fn take(&mut self) -> Option<SseEvent> {
        if self.data.is_empty() && self.event.is_none() {
            return None;
        }
        Some(SseEvent {
            event: self.event.take(),
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events_across_chunk_boundaries() {
        let mut p = SseParser::new();
        let mut evs = p.push(b"event: a\ndata: {\"x\"");
        assert!(evs.is_empty());
        evs.extend(p.push(b":1}\r\n\r\ndata: two\n"));
        evs.extend(p.push(b"data: lines\n\n: ping\n\n"));
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].event.as_deref(), Some("a"));
        assert_eq!(evs[0].data, "{\"x\":1}");
        assert_eq!(evs[1].data, "two\nlines");
    }

    #[test]
    fn utf8_split_mid_char() {
        let mut p = SseParser::new();
        let s = "data: héllo\n\n".as_bytes();
        let mut evs = p.push(&s[..8]);
        evs.extend(p.push(&s[8..]));
        assert_eq!(evs[0].data, "héllo");
    }

    #[test]
    fn finish_flushes_unterminated() {
        let mut p = SseParser::new();
        assert!(p.push(b"data: x").is_empty());
        assert_eq!(p.finish().unwrap().data, "x");
    }
}

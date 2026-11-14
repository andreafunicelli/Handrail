//! Minimal Server-Sent Events parser (line based, tolerant of `\r\n` and chunk splits).

use crate::error::Error;

/// Most bytes one event may buffer (a line without a newline, or many `data:` lines without the
/// blank line that ends the event). Real events are far smaller; past this the stream fails.
pub const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;

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
    data_bytes: usize,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed raw bytes; returns every event completed by them, or an error once an event grows
    /// past [`MAX_EVENT_BYTES`].
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, Error> {
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
                    "data" => {
                        self.data_bytes += value.len();
                        self.data.push(value.to_string());
                    }
                    _ => {}
                }
            }
        }
        if self.buf.len() + self.data_bytes > MAX_EVENT_BYTES {
            return Err(Error::Protocol(format!(
                "server-sent event larger than {} MB",
                MAX_EVENT_BYTES / (1024 * 1024)
            )));
        }
        Ok(out)
    }

    /// Flush a trailing event not terminated by a blank line.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            self.buf.push(b'\n');
            // At most the buffer already accepted plus a newline: can only fail if it was full.
            self.push(&[]).ok()?;
        }
        self.take()
    }

    fn take(&mut self) -> Option<SseEvent> {
        if self.data.is_empty() && self.event.is_none() {
            return None;
        }
        self.data_bytes = 0;
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
        let mut evs = p.push(b"event: a\ndata: {\"x\"").unwrap();
        assert!(evs.is_empty());
        evs.extend(p.push(b":1}\r\n\r\ndata: two\n").unwrap());
        evs.extend(p.push(b"data: lines\n\n: ping\n\n").unwrap());
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].event.as_deref(), Some("a"));
        assert_eq!(evs[0].data, "{\"x\":1}");
        assert_eq!(evs[1].data, "two\nlines");
    }

    #[test]
    fn utf8_split_mid_char() {
        let mut p = SseParser::new();
        let s = "data: héllo\n\n".as_bytes();
        let mut evs = p.push(&s[..8]).unwrap();
        evs.extend(p.push(&s[8..]).unwrap());
        assert_eq!(evs[0].data, "héllo");
    }

    #[test]
    fn finish_flushes_unterminated() {
        let mut p = SseParser::new();
        assert!(p.push(b"data: x").unwrap().is_empty());
        assert_eq!(p.finish().unwrap().data, "x");
    }

    #[test]
    fn oversized_events_fail_the_stream() {
        let chunk = vec![b'a'; 1024 * 1024];
        let mut p = SseParser::new();
        p.push(b"data: ").unwrap();
        let fails = (0..10)
            .map(|_| p.push(&chunk))
            .filter(|r| r.is_err())
            .count();
        assert!(fails > 0, "a line with no newline is capped");
        let mut p = SseParser::new();
        let line = [b"data: ".as_slice(), &chunk, b"\n"].concat();
        assert!(
            (0..10).any(|_| p.push(&line).is_err()),
            "an event never ended is capped"
        );
        // Ordinary events reset the count.
        let mut p = SseParser::new();
        let ev = [b"data: ".as_slice(), &chunk, b"\n\n"].concat();
        for _ in 0..20 {
            assert_eq!(p.push(&ev).unwrap().len(), 1);
        }
    }
}

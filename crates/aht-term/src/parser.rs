//! Streaming terminal output analysis.
//!
//! * [`Tracker`] runs the `vte` state machine over raw output and reports semantic events
//!   (OSC 133 marks, OSC 7 cwd, alt-screen, our private OSC 1337 sentinels) plus a dumb
//!   single-line text model that yields ANSI-stripped committed lines.
//! * [`UiFilter`] is a byte-level pass that removes our private `OSC 1337;Aht...` sequences from
//!   the stream sent to the UI and implements the hidden-injection suppression window.
//!
//! Both are chunk-boundary safe: escape sequences and UTF-8 split across reads are handled.

/// Semantic events found in terminal output, in stream order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackEvent {
    /// A completed (newline-terminated) line of plain text.
    Line(String),
    PromptStart,
    PromptEnd,
    CommandStart,
    CommandEnd(Option<i32>),
    Cwd(String),
    AltScreen(bool),
    SentinelBegin(String),
    SentinelEnd(String),
    /// `OSC 1337;AhtKV=key=value`
    Kv(String, String),
}

#[derive(Default)]
struct Perf {
    events: Vec<TrackEvent>,
    line: Vec<char>,
    col: usize,
    alt: bool,
}

impl Perf {
    fn put_char(&mut self, c: char) {
        if self.col < self.line.len() {
            self.line[self.col] = c;
        } else {
            while self.line.len() < self.col {
                self.line.push(' ');
            }
            self.line.push(c);
        }
        self.col += 1;
    }

    fn commit(&mut self) {
        let s: String = self.line.iter().collect();
        self.events.push(TrackEvent::Line(s.trim_end().to_string()));
        self.line.clear();
        self.col = 0;
    }
}

fn param(params: &vte::Params, idx: usize, default: u16) -> usize {
    params
        .iter()
        .nth(idx)
        .and_then(|p| p.first().copied())
        .filter(|&v| v != 0)
        .unwrap_or(default) as usize
}

impl vte::Perform for Perf {
    fn print(&mut self, c: char) {
        if !self.alt {
            self.put_char(c);
        }
    }

    fn execute(&mut self, byte: u8) {
        if self.alt {
            return;
        }
        match byte {
            b'\n' => self.commit(),
            b'\r' => self.col = 0,
            0x08 => self.col = self.col.saturating_sub(1),
            b'\t' => {
                let next = (self.col / 8 + 1) * 8;
                while self.col < next {
                    self.put_char(' ');
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        if intermediates == b"?" {
            if matches!(action, 'h' | 'l') {
                let alt_mode = params
                    .iter()
                    .any(|p| matches!(p.first(), Some(1049 | 47 | 1047)));
                if alt_mode {
                    let on = action == 'h';
                    if on != self.alt {
                        self.alt = on;
                        self.events.push(TrackEvent::AltScreen(on));
                    }
                }
            }
            return;
        }
        if self.alt || !intermediates.is_empty() {
            return;
        }
        match action {
            'K' => match param(params, 0, 0) {
                0 => self.line.truncate(self.col),
                1 => {
                    for i in 0..self.col.min(self.line.len()) {
                        self.line[i] = ' ';
                    }
                }
                _ => self.line.clear(),
            },
            'C' => self.col += param(params, 0, 1),
            'D' => self.col = self.col.saturating_sub(param(params, 0, 1)),
            'G' => self.col = param(params, 0, 1) - 1,
            'H' | 'f' => self.col = param(params, 1, 1) - 1,
            'P' => {
                let n = param(params, 0, 1);
                if self.col < self.line.len() {
                    let end = (self.col + n).min(self.line.len());
                    self.line.drain(self.col..end);
                }
            }
            '@' => {
                let n = param(params, 0, 1);
                if self.col < self.line.len() {
                    for _ in 0..n {
                        self.line.insert(self.col, ' ');
                    }
                }
            }
            'X' => {
                let n = param(params, 0, 1);
                for i in self.col..(self.col + n).min(self.line.len()) {
                    self.line[i] = ' ';
                }
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        let Some(&first) = params.first() else {
            return;
        };
        let text = |i: usize| {
            params
                .get(i)
                .map(|p| String::from_utf8_lossy(p).into_owned())
        };
        match first {
            b"133" | b"633" => match params.get(1).and_then(|p| p.first()) {
                Some(b'A') => self.events.push(TrackEvent::PromptStart),
                Some(b'B') => self.events.push(TrackEvent::PromptEnd),
                Some(b'C') => self.events.push(TrackEvent::CommandStart),
                Some(b'D') => {
                    let code = text(2).and_then(|s| s.trim().parse().ok());
                    self.events.push(TrackEvent::CommandEnd(code));
                }
                _ => {}
            },
            b"7" => {
                let joined = params[1..]
                    .iter()
                    .map(|p| String::from_utf8_lossy(p))
                    .collect::<Vec<_>>()
                    .join(";");
                if let Some(path) = parse_osc7(&joined) {
                    self.events.push(TrackEvent::Cwd(path));
                }
            }
            b"1337" => {
                let rest = params[1..]
                    .iter()
                    .map(|p| String::from_utf8_lossy(p))
                    .collect::<Vec<_>>()
                    .join(";");
                if let Some(n) = rest.strip_prefix("AhtBegin=") {
                    self.events.push(TrackEvent::SentinelBegin(n.to_string()));
                } else if let Some(n) = rest.strip_prefix("AhtEnd=") {
                    self.events.push(TrackEvent::SentinelEnd(n.to_string()));
                } else if let Some(kv) = rest.strip_prefix("AhtKV=") {
                    if let Some((k, v)) = kv.split_once('=') {
                        self.events
                            .push(TrackEvent::Kv(k.to_string(), v.trim().to_string()));
                    }
                }
            }
            _ => {}
        }
    }
}

/// `file://host/path` → `/path` (percent-decoded). Accepts bare paths too.
pub fn parse_osc7(s: &str) -> Option<String> {
    let path = if let Some(rest) = s.strip_prefix("file://") {
        &rest[rest.find('/')?..]
    } else if s.starts_with('/') {
        s
    } else {
        return None;
    };
    Some(percent_decode(path))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Streaming semantic tracker over raw terminal output.
pub struct Tracker {
    parser: vte::Parser,
    perf: Perf,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Tracker {
    pub fn new() -> Self {
        Self {
            parser: vte::Parser::new(),
            perf: Perf::default(),
        }
    }

    /// Feed raw bytes; returns events in stream order.
    pub fn feed(&mut self, data: &[u8]) -> Vec<TrackEvent> {
        self.parser.advance(&mut self.perf, data);
        std::mem::take(&mut self.perf.events)
    }

    /// The current (not yet newline-terminated) line, ANSI-stripped, trailing spaces kept.
    pub fn current_line(&self) -> String {
        self.perf.line.iter().collect()
    }

    pub fn in_alt_screen(&self) -> bool {
        self.perf.alt
    }
}

/// Byte-level UI filter: strips `OSC 1337;Aht…` sequences and suppresses output while hidden.
#[derive(Default)]
pub struct UiFilter {
    state: FState,
    buf: Vec<u8>,
    hidden: bool,
    end_nonce: Option<String>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum FState {
    #[default]
    Ground,
    Esc,
    Osc,
    OscEsc,
}

const MAX_OSC: usize = 8192;
/// Sent to the UI when a hidden window ends: return to column 0 and clear the line, so the
/// prompt that follows overwrites the prompt that was on screen before the injection.
pub const CLEAR_LINE: &[u8] = b"\r\x1b[K";

impl UiFilter {
    /// Start suppressing all output until `OSC 1337;AhtEnd=<nonce>` (or [`UiFilter::release`]).
    pub fn hide_until(&mut self, nonce: Option<String>) {
        self.hidden = true;
        self.end_nonce = nonce;
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// End suppression explicitly.
    pub fn release(&mut self) {
        self.hidden = false;
        self.end_nonce = None;
    }

    pub fn filter(&mut self, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len());
        for &b in data {
            self.byte(b, &mut out);
        }
        out
    }

    fn emit(&self, out: &mut Vec<u8>, bytes: &[u8]) {
        if !self.hidden {
            out.extend_from_slice(bytes);
        }
    }

    fn byte(&mut self, b: u8, out: &mut Vec<u8>) {
        match self.state {
            FState::Ground => {
                if b == 0x1b {
                    self.buf.clear();
                    self.buf.push(b);
                    self.state = FState::Esc;
                } else {
                    self.emit(out, &[b]);
                }
            }
            FState::Esc => {
                if b == b']' {
                    self.buf.push(b);
                    self.state = FState::Osc;
                } else {
                    let buf = std::mem::take(&mut self.buf);
                    self.emit(out, &buf);
                    self.state = FState::Ground;
                    self.byte(b, out);
                }
            }
            FState::Osc => {
                if b == 0x07 {
                    self.buf.push(b);
                    self.finish_osc(out, 1);
                } else if b == 0x1b {
                    self.state = FState::OscEsc;
                } else {
                    self.buf.push(b);
                    if self.buf.len() > MAX_OSC {
                        let buf = std::mem::take(&mut self.buf);
                        self.emit(out, &buf);
                        self.state = FState::Ground;
                    }
                }
            }
            FState::OscEsc => {
                if b == b'\\' {
                    self.buf.extend_from_slice(b"\x1b\\");
                    self.finish_osc(out, 2);
                } else {
                    // ESC aborted the OSC; pass it through and restart on the new ESC.
                    let buf = std::mem::take(&mut self.buf);
                    self.emit(out, &buf);
                    self.buf.push(0x1b);
                    self.state = FState::Esc;
                    self.byte(b, out);
                }
            }
        }
    }

    fn finish_osc(&mut self, out: &mut Vec<u8>, term_len: usize) {
        self.state = FState::Ground;
        let buf = std::mem::take(&mut self.buf);
        let body = &buf[2..buf.len() - term_len];
        if let Some(rest) = body.strip_prefix(b"1337;Aht") {
            if let (true, Some(nonce)) = (self.hidden, &self.end_nonce) {
                if rest.strip_prefix(b"End=") == Some(nonce.as_bytes()) {
                    self.release();
                    out.extend_from_slice(CLEAR_LINE);
                }
            }
            return;
        }
        self.emit(out, &buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_split(data: &[u8]) -> Vec<TrackEvent> {
        // Feed byte by byte to exercise every chunk boundary.
        let mut t = Tracker::new();
        let mut ev = Vec::new();
        for b in data {
            ev.extend(t.feed(std::slice::from_ref(b)));
        }
        ev
    }

    #[test]
    fn marks_split_across_chunks() {
        let data = "\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07a b\r\n\x1b]133;D;2\x1b\\\x1b]7;file://h/tmp/a%20b\x07";
        let ev = feed_split(data.as_bytes());
        assert_eq!(
            ev,
            vec![
                TrackEvent::PromptStart,
                TrackEvent::PromptEnd,
                TrackEvent::Line("$ ls".into()),
                TrackEvent::CommandStart,
                TrackEvent::Line("a b".into()),
                TrackEvent::CommandEnd(Some(2)),
                TrackEvent::Cwd("/tmp/a b".into()),
            ]
        );
    }

    #[test]
    fn alt_screen_and_utf8_split() {
        let mut t = Tracker::new();
        let s = "héllo \x1b[?1049hJUNK\r\n\x1b[?1049lend\r\n".as_bytes();
        let (a, b) = s.split_at(2); // splits inside 'é'
        let mut ev = t.feed(a);
        ev.extend(t.feed(b));
        assert_eq!(
            ev,
            vec![
                TrackEvent::AltScreen(true),
                TrackEvent::AltScreen(false),
                TrackEvent::Line("héllo end".into()),
            ]
        );
    }

    #[test]
    fn line_model_handles_cr_and_erase() {
        let mut t = Tracker::new();
        let ev = t.feed(b"\x1b[32m10%\x1b[0m\r50%\r100%\r\nab\x08c\r\nxyz\r\x1b[Kq\r\n");
        assert_eq!(
            ev,
            vec![
                TrackEvent::Line("100%".into()),
                TrackEvent::Line("ac".into()),
                TrackEvent::Line("q".into())
            ]
        );
        t.feed(b"user@h:~$ ");
        assert_eq!(t.current_line(), "user@h:~$ ");
    }

    #[test]
    fn private_sentinels() {
        let ev = feed_split(b"\x1b]1337;AhtBegin=n1\x07\x1b]1337;AhtKV=os=Ubuntu 22.04; LTS\x07\x1b]1337;AhtEnd=n1\x07");
        assert_eq!(
            ev,
            vec![
                TrackEvent::SentinelBegin("n1".into()),
                TrackEvent::Kv("os".into(), "Ubuntu 22.04; LTS".into()),
                TrackEvent::SentinelEnd("n1".into()),
            ]
        );
    }

    #[test]
    fn ui_filter_strips_private_osc_split() {
        let mut f = UiFilter::default();
        let data = b"a\x1b]1337;AhtKV=x=y\x07b\x1b]133;A\x07c\x1b[1md";
        let mut out = Vec::new();
        for b in data {
            out.extend(f.filter(std::slice::from_ref(b)));
        }
        assert_eq!(out, b"ab\x1b]133;A\x07c\x1b[1md");
    }

    #[test]
    fn ui_filter_hidden_window() {
        let mut f = UiFilter::default();
        f.hide_until(Some("N".into()));
        let mut out =
            f.filter(b" printf '\\033]1337;AhtEnd=N\\007'\r\n\x1b]1337;AhtEnd=X\x07still hidden");
        assert!(out.is_empty());
        out = f.filter(b"\x1b]1337;AhtEnd=N\x07$ ");
        assert_eq!(out, b"\r\x1b[K$ ");
        assert!(!f.is_hidden());
    }
}

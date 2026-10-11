//! In-memory fake shell implementing `Transport`: a tiny bash-like line editor + interpreter.
#![allow(dead_code)]

use aht_core::agent::{ApprovalDecision, ApprovalRequest};
use aht_core::terminal::{Transport, TransportEvent};
use aht_core::Result;
use aht_term::Approver;
use async_trait::async_trait;
use regex::Regex;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Bash-like: answers the probe as bash and honours the injected hook.
    Bash,
    /// POSIX sh without integration (Limited mode).
    Dumb,
}

#[derive(PartialEq, Eq, Debug)]
enum Mode {
    Prompt,
    Running,
    Password(String),
    Vim,
}

struct St {
    kind: Kind,
    line: String,
    mode: Mode,
    hooked: bool,
    ran: bool,
    last_ec: i32,
    gen: u64,
    pub written: Vec<u8>,
}

pub struct FakeShell {
    st: Mutex<St>,
    tx: mpsc::Sender<TransportEvent>,
    chunk: usize,
}

impl FakeShell {
    pub fn new(kind: Kind) -> (Arc<FakeShell>, mpsc::Receiver<TransportEvent>) {
        Self::with_chunk(kind, 7)
    }

    /// `chunk` = max bytes per emitted `Data` event.
    pub fn with_chunk(
        kind: Kind,
        chunk: usize,
    ) -> (Arc<FakeShell>, mpsc::Receiver<TransportEvent>) {
        let (tx, rx) = mpsc::channel(100_000);
        let sh = Arc::new(FakeShell {
            st: Mutex::new(St {
                kind,
                line: String::new(),
                mode: Mode::Prompt,
                hooked: false,
                ran: false,
                last_ec: 0,
                gen: 0,
                written: Vec::new(),
            }),
            tx,
            chunk,
        });
        // Login banner + first prompt.
        let s2 = sh.clone();
        let banner = "Welcome to FakeOS\r\n".to_string();
        let p = s2.prompt_bytes(&mut s2.st.lock().unwrap());
        s2.emit(banner.as_bytes());
        s2.emit(&p);
        (sh, rx)
    }

    pub fn written(&self) -> String {
        String::from_utf8_lossy(&self.st.lock().unwrap().written).into_owned()
    }

    pub fn hooked(&self) -> bool {
        self.st.lock().unwrap().hooked
    }

    pub fn disconnect(&self) {
        let _ = self
            .tx
            .try_send(TransportEvent::Closed { exit_status: None });
    }

    /// Deliver a raw transport event (how a real connection reports its end).
    pub fn event(&self, ev: TransportEvent) {
        let _ = self.tx.try_send(ev);
    }

    /// Emit output split into small chunks to exercise split-sequence parsing.
    fn emit(&self, data: &[u8]) {
        for c in data.chunks(self.chunk) {
            let _ = self.tx.try_send(TransportEvent::Data(c.to_vec()));
        }
    }

    fn prompt_bytes(&self, st: &mut St) -> Vec<u8> {
        let mut out = Vec::new();
        match st.kind {
            Kind::Bash => {
                if st.hooked {
                    if st.ran {
                        out.extend(format!("\x1b]133;D;{}\x07", st.last_ec).bytes());
                    }
                    out.extend(b"\x1b]7;file://fake/home/u\x07\x1b]133;A\x07");
                }
                out.extend(b"\x1b[32mu@fake\x1b[0m:~$ ");
                if st.hooked {
                    out.extend(b"\x1b]133;B\x07");
                }
            }
            Kind::Dumb => out.extend(b"$ "),
        }
        st.ran = false;
        out
    }

    fn finish(self: &Arc<Self>, ec: i32) {
        let mut st = self.st.lock().unwrap();
        st.mode = Mode::Prompt;
        st.last_ec = ec;
        let p = self.prompt_bytes(&mut st);
        drop(st);
        self.emit(&p);
    }

    fn run(self: &Arc<Self>, line: String) {
        let line = line.trim().to_string();
        let kind = self.st.lock().unwrap().kind;
        if line.is_empty() {
            return self.finish(self.st.lock().unwrap().last_ec);
        }
        {
            let mut st = self.st.lock().unwrap();
            if st.hooked {
                st.ran = true;
                drop(st);
                self.emit(b"\x1b]133;C\x07");
            }
        }
        if line.starts_with("echo \"AHTPROBE") {
            let out = match kind {
                Kind::Bash => "AHTPROBE b=5.2.15(1)-release z= f= p= e=END\r\n",
                Kind::Dumb => "AHTPROBE b= z= f= p= e=END\r\n",
            };
            self.emit(out.as_bytes());
            return self.finish(0);
        }
        if line.contains("AhtBegin=") {
            let re = Regex::new(r"AhtBegin=%s\\007' (\w+);").unwrap();
            if let (Kind::Bash, Some(c)) = (kind, re.captures(&line)) {
                let n = c[1].to_string();
                let out = format!(
                    "\x1b]1337;AhtBegin={n}\x07\x1b]1337;AhtKV=shell=bash\x07\x1b]1337;AhtKV=user=u\x07\x1b]1337;AhtKV=host=fake\x07\x1b]1337;AhtKV=os=FakeOS 1.0\x07\x1b]1337;AhtEnd={n}\x07"
                );
                self.emit(out.as_bytes());
                let mut st = self.st.lock().unwrap();
                st.hooked = true;
                st.ran = false;
                drop(st);
                return self.finish(0);
            }
            self.emit(b"sh: syntax error\r\n");
            return self.finish(2);
        }
        let (cmd, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match cmd {
            "echo" => {
                self.emit(format!("{rest}\r\n").as_bytes());
                self.finish(0)
            }
            "ls" => {
                self.emit(b"\x1b[34ma.txt\x1b[0m  b.txt\r\n");
                self.finish(0)
            }
            "true" => self.finish(0),
            "false" => self.finish(1),
            "exit" => self.disconnect(),
            "sleep" => {
                let secs: f64 = rest.parse().unwrap_or(1.0);
                let me = self.clone();
                let gen = {
                    let mut st = self.st.lock().unwrap();
                    st.mode = Mode::Running;
                    st.gen += 1;
                    st.gen
                };
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
                    let still = {
                        let st = me.st.lock().unwrap();
                        st.mode == Mode::Running && st.gen == gen
                    };
                    if still {
                        me.emit(b"slept\r\n");
                        me.finish(0);
                    }
                });
            }
            "sudo" => {
                self.st.lock().unwrap().mode = Mode::Password(rest.to_string());
                self.emit(b"[sudo] password for u: ");
            }
            "vim" => {
                self.st.lock().unwrap().mode = Mode::Vim;
                self.emit(b"\x1b[?1049h\x1b[H\x1b[2J~\r\n~\r\n\"file\" 0L");
            }
            "cat" if rest == "bigfile" => {
                let mut out = String::new();
                let mut i = 0;
                while out.len() < 100 * 1024 {
                    out.push_str(&format!("line {i:06} {}\r\n", "x".repeat(40)));
                    i += 1;
                }
                self.emit(out.as_bytes());
                self.finish(0)
            }
            "printf" => {
                // printf without newline at end
                self.emit(rest.trim_matches('\'').as_bytes());
                self.finish(0)
            }
            _ => {
                self.emit(format!("bash: {cmd}: command not found\r\n").as_bytes());
                self.finish(127)
            }
        }
    }

    fn input(self: &Arc<Self>, data: &[u8]) {
        for &b in data {
            let mode_prompt;
            {
                let st = self.st.lock().unwrap();
                mode_prompt = match &st.mode {
                    Mode::Prompt => 0,
                    Mode::Running => 1,
                    Mode::Password(_) => 2,
                    Mode::Vim => 3,
                };
            }
            match mode_prompt {
                0 => match b {
                    b'\r' => {
                        let line = std::mem::take(&mut self.st.lock().unwrap().line);
                        self.emit(b"\r\n");
                        self.run(line);
                    }
                    0x03 => {
                        self.st.lock().unwrap().line.clear();
                        self.emit(b"^C\r\n");
                        self.finish(130);
                    }
                    0x7f => {
                        if self.st.lock().unwrap().line.pop().is_some() {
                            self.emit(b"\x08 \x08");
                        }
                    }
                    _ => {
                        self.st.lock().unwrap().line.push(b as char);
                        self.emit(&[b]);
                    }
                },
                1 => {
                    if b == 0x03 {
                        self.emit(b"^C\r\n");
                        self.finish(130);
                    } else {
                        self.emit(&[b]); // tty echo while a program runs
                    }
                }
                2 => {
                    if b == b'\r' {
                        let cmd = match std::mem::replace(
                            &mut self.st.lock().unwrap().mode,
                            Mode::Prompt,
                        ) {
                            Mode::Password(c) => c,
                            _ => String::new(),
                        };
                        self.emit(b"\r\n");
                        // Run the inner command as if typed (without a new C mark).
                        self.run_inner(cmd);
                    } else if b == 0x03 {
                        self.st.lock().unwrap().mode = Mode::Prompt;
                        self.emit(b"\r\n");
                        self.finish(1);
                    }
                }
                _ => {
                    let mut st = self.st.lock().unwrap();
                    st.line.push(b as char);
                    if st.line.ends_with(":q\r") {
                        st.line.clear();
                        drop(st);
                        self.emit(b"\x1b[?1049l");
                        self.finish(0);
                    }
                }
            }
        }
    }

    fn run_inner(self: &Arc<Self>, cmd: String) {
        let (c, rest) = cmd.split_once(' ').unwrap_or((cmd.as_str(), ""));
        match c {
            "echo" => {
                self.emit(format!("{rest}\r\n").as_bytes());
                self.finish(0)
            }
            _ => self.finish(0),
        }
    }
}

/// `Transport` handle over a fake shell.
pub struct FakeHandle(pub Arc<FakeShell>);

#[async_trait]
impl Transport for FakeHandle {
    async fn write(&self, data: &[u8]) -> Result<()> {
        self.0.st.lock().unwrap().written.extend_from_slice(data);
        self.0.input(data);
        Ok(())
    }
    async fn resize(&self, _cols: u16, _rows: u16) -> Result<()> {
        Ok(())
    }
    async fn close(&self) -> Result<()> {
        self.0.disconnect();
        Ok(())
    }
}

/// Approver returning a fixed decision and recording requests.
pub struct TestApprover {
    pub decision: ApprovalDecision,
    pub calls: Mutex<Vec<ApprovalRequest>>,
    pub delay: Duration,
}

impl TestApprover {
    pub fn new(decision: ApprovalDecision) -> Self {
        Self {
            decision,
            calls: Mutex::new(Vec::new()),
            delay: Duration::ZERO,
        }
    }
    pub fn approve() -> Self {
        Self::new(ApprovalDecision::Approve)
    }
    pub fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[async_trait]
impl Approver for TestApprover {
    async fn request(&self, req: ApprovalRequest) -> ApprovalDecision {
        self.calls.lock().unwrap().push(req);
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.decision.clone()
    }
}

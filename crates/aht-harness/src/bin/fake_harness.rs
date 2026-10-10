//! Test double for a harness CLI (built with the `test-support` feature).
//!
//! Usage: `fake_harness <fixture.jsonl> [options]`
//! * `--stdin-to=<path>`   write everything read from stdin (until EOF) to that file first
//! * `--args-to=<path>`    write argv (one per line) to that file
//! * `--stderr=<text>`     print a line to stderr before replaying
//! * `--exit=<n>`          exit code after replay (default 0)
//! * `--hang`              after replay, sleep forever (cancel test)
//! * `--spawn-child`       spawn a sleeping grandchild (this binary with `--hang` and no fixture)
//! * `--claude-echo`       ignore the fixture; for each stdin JSON line print an assistant event
//!   echoing the user text plus a result event; exit at stdin EOF
//! * `--print-env=<NAME>`  print `{"type":"error","message":"ENV <NAME>=<value|unset>"}` first

use std::io::{BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |name: &str| {
        args.iter()
            .find_map(|a| a.strip_prefix(&format!("--{name}=")).map(str::to_string))
    };
    let flag = |name: &str| args.iter().any(|a| a == &format!("--{name}"));
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if flag("claude-echo") {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
            let text = v["message"]["content"][0]["text"].as_str().unwrap_or("?");
            let ev = serde_json::json!({"type":"assistant","message":{"content":[
                {"type":"text","text":format!("echo: {text}")}]}});
            let res = serde_json::json!({"type":"result","subtype":"success","is_error":false,
                "usage":{"input_tokens":1,"output_tokens":1},"total_cost_usd":0.0});
            let _ = writeln!(out, "{ev}");
            let _ = writeln!(out, "{res}");
            let _ = out.flush();
        }
        return;
    }

    if let Some(path) = opt("args-to") {
        let _ = std::fs::write(
            path,
            args.join(
                "
",
            ),
        );
    }
    if let Some(path) = opt("stdin-to") {
        let mut buf = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf);
        let _ = std::fs::write(path, buf);
    }
    if let Some(t) = opt("stderr") {
        eprintln!("{t}");
    }
    if let Some(name) = opt("print-env") {
        let val = std::env::var(&name).unwrap_or_else(|_| "unset".into());
        let _ = writeln!(
            out,
            "{}",
            serde_json::json!({"type":"error","message":format!("ENV {name}={val}")})
        );
    }
    if flag("spawn-child") {
        let me = std::env::current_exe().expect("exe");
        #[allow(clippy::zombie_processes)] // intentionally orphaned: the test kills the tree
        let child = std::process::Command::new(me)
            .arg("--hang")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("grandchild");
        let _ = writeln!(
            out,
            "{}",
            serde_json::json!({"type":"error","message":format!("CHILD {}", child.id())})
        );
        let _ = out.flush();
    }
    if let Some(fixture) = args.iter().find(|a| !a.starts_with("--")) {
        let text = std::fs::read_to_string(fixture).expect("fixture");
        for l in text.lines() {
            let _ = writeln!(out, "{l}");
            let _ = out.flush();
        }
    }
    let _ = out.flush();
    if flag("hang") {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    std::process::exit(opt("exit").and_then(|s| s.parse().ok()).unwrap_or(0));
}

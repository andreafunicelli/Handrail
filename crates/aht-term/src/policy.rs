//! Approval policy engine: risk classification of shell command lines and the decision of
//! whether a command needs the user's approval.

use aht_core::agent::RiskClass;
use aht_core::model::{ApprovalMode, Environment};
use regex::Regex;
use std::sync::OnceLock;

/// Per-session approval policy.
#[derive(Clone, Debug)]
pub struct Policy {
    pub mode: ApprovalMode,
    pub env: Environment,
    /// Commands matching any of these always require approval.
    pub deny_list: Vec<Regex>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            mode: ApprovalMode::ConfirmAll,
            env: Environment::None,
            deny_list: default_deny_list(),
        }
    }
}

/// A conservative built-in deny-list. Callers may extend or replace it.
pub fn default_deny_list() -> Vec<Regex> {
    [
        r"\brm\s+(-[a-zA-Z]*\s+)*-[a-zA-Z]*[rR][a-zA-Z]*\s+(-[a-zA-Z]*\s+)*/(\s|$|\*)",
        r"\bmkfs(\.\w+)?\b",
        r"\bdd\b.*\bof=/dev/",
        r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:",
        r"(?i)\bdrop\s+(database|schema)\b",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("valid default deny regex"))
    .collect()
}

/// Outcome of evaluating a command against a [`Policy`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyVerdict {
    pub risk: RiskClass,
    pub needs_approval: bool,
    /// Why approval is required (or `None` when it runs without approval).
    pub note: Option<String>,
}

impl Policy {
    /// The mode actually enforced: `Auto` on a `Prod` connection is downgraded to `ConfirmWrites`
    /// (the app requires an explicit, typed opt-in elsewhere; this is a second line of defense).
    pub fn effective_mode(&self) -> ApprovalMode {
        match (self.mode, self.env) {
            (ApprovalMode::Auto, Environment::Prod) => ApprovalMode::ConfirmWrites,
            (m, _) => m,
        }
    }

    pub fn evaluate(&self, command: &str) -> PolicyVerdict {
        let risk = classify(command);
        if let Some(re) = self.deny_list.iter().find(|r| r.is_match(command)) {
            return PolicyVerdict {
                risk,
                needs_approval: true,
                note: Some(format!("matches deny-list: {}", re.as_str())),
            };
        }
        let mode = self.effective_mode();
        let downgraded = mode != self.mode;
        let (needs, note) = match mode {
            ApprovalMode::ConfirmAll => (true, "approval mode: confirm all commands".to_string()),
            ApprovalMode::ConfirmWrites => match risk {
                RiskClass::Read => (false, String::new()),
                RiskClass::Write => (true, "command may modify the system".to_string()),
                RiskClass::Destructive => (true, "destructive command".to_string()),
            },
            ApprovalMode::Auto => match risk {
                RiskClass::Destructive => (true, "destructive command".to_string()),
                _ => (false, String::new()),
            },
        };
        let note = needs.then(|| {
            if downgraded {
                format!("{note} (production: auto mode treated as confirm-writes)")
            } else {
                note
            }
        });
        PolicyVerdict {
            risk,
            needs_approval: needs,
            note,
        }
    }
}

fn max_risk(a: RiskClass, b: RiskClass) -> RiskClass {
    fn rank(r: RiskClass) -> u8 {
        match r {
            RiskClass::Read => 0,
            RiskClass::Write => 1,
            RiskClass::Destructive => 2,
        }
    }
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

/// Classify a full command line. Compound commands (pipes, `;`, `&&`, `||`, `&`, subshells,
/// command substitution) are split and the maximum risk of all parts is returned.
pub fn classify(command: &str) -> RiskClass {
    let mut risk = RiskClass::Read;
    if global_destructive().is_match(command) {
        return RiskClass::Destructive;
    }
    let segments = split_segments(command);
    if segments.iter().all(|s| s.words.is_empty() && s.redirects.is_empty()) {
        return RiskClass::Read;
    }
    for seg in &segments {
        risk = max_risk(risk, classify_segment(seg));
        if risk == RiskClass::Destructive {
            break;
        }
    }
    risk
}

/// Patterns checked against the whole raw line (they may appear inside quoted arguments, e.g.
/// `mysql -e "DROP TABLE x"`).
fn global_destructive() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"(?i)\bdrop\s+(database|table|schema|user)\b",
            r"|(?i)\btruncate\s+table\b",
            r"|(?i)\bdelete\s+from\s+\w+\s*(;|$|\x22|')",
            r"|:\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}",
            r"|>\s*/dev/(sd|hd|vd|xvd|nvme|mmcblk|disk)\w*",
        ))
        .expect("valid regex")
    })
}

#[derive(Debug, Default)]
struct Segment {
    words: Vec<String>,
    /// Redirection targets (`> file`, `>> file`, `&> file`); `None` target means fd dup (`2>&1`).
    redirects: Vec<String>,
}

/// Quote-aware split of a command line into simple-command segments.
fn split_segments(line: &str) -> Vec<Segment> {
    let mut segs = Vec::new();
    let mut cur = Segment::default();
    let mut word = String::new();
    let mut in_word = false;
    let mut pending_redirect = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    fn finish_word(
        word: &mut String,
        in_word: &mut bool,
        pending_redirect: &mut bool,
        cur: &mut Segment,
    ) {
        if *in_word {
            if *pending_redirect {
                cur.redirects.push(std::mem::take(word));
                *pending_redirect = false;
            } else {
                cur.words.push(std::mem::take(word));
            }
            *in_word = false;
        }
    }

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    word.push(chars[i]);
                    i += 1;
                }
            }
            '"' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    } else if chars[i] == '$' && chars.get(i + 1) == Some(&'(') {
                        // Command substitution inside double quotes: classify it as its own part.
                        let (inner, end) = take_balanced(&chars, i + 2);
                        segs.extend(split_segments(&inner));
                        i = end;
                        continue;
                    } else if chars[i] == '`' {
                        let end = chars[i + 1..]
                            .iter()
                            .position(|&x| x == '`')
                            .map_or(chars.len(), |p| i + 1 + p);
                        let inner: String = chars[i + 1..end].iter().collect();
                        segs.extend(split_segments(&inner));
                        i = end + 1;
                        continue;
                    }
                    word.push(chars[i]);
                    i += 1;
                }
            }
            '\\' => {
                in_word = true;
                if i + 1 < chars.len() {
                    word.push(chars[i + 1]);
                    i += 1;
                }
            }
            ' ' | '\t' => finish_word(&mut word, &mut in_word, &mut pending_redirect, &mut cur),
            ';' | '|' | '&' | '\n' | '(' | ')' | '`' | '{' | '}' => {
                // `&>` and `>&` are redirections, not separators.
                if c == '&' && chars.get(i + 1) == Some(&'>') {
                    finish_word(&mut word, &mut in_word, &mut pending_redirect, &mut cur);
                    i += 1;
                    if chars.get(i + 1) == Some(&'>') {
                        i += 1;
                    }
                    pending_redirect = true;
                } else if (c == '{' || c == '}') && in_word {
                    word.push(c);
                } else {
                    finish_word(&mut word, &mut in_word, &mut pending_redirect, &mut cur);
                    pending_redirect = false;
                    if !cur.words.is_empty() || !cur.redirects.is_empty() {
                        segs.push(std::mem::take(&mut cur));
                    }
                }
            }
            '$' if chars.get(i + 1) == Some(&'(') => {
                finish_word(&mut word, &mut in_word, &mut pending_redirect, &mut cur);
                let (inner, end) = take_balanced(&chars, i + 2);
                segs.extend(split_segments(&inner));
                // The substitution's result acts as a word in the outer command.
                word.push_str("$SUBST");
                in_word = true;
                i = end;
                continue;
            }
            '>' | '<' => {
                // Digits immediately before the operator are an fd number, not a word.
                if in_word && word.chars().all(|ch| ch.is_ascii_digit()) {
                    word.clear();
                    in_word = false;
                }
                finish_word(&mut word, &mut in_word, &mut pending_redirect, &mut cur);
                let is_out = c == '>';
                if chars.get(i + 1) == Some(&'>') {
                    i += 1;
                }
                if chars.get(i + 1) == Some(&'&') {
                    // `2>&1`: fd duplication; skip the target fd.
                    i += 2;
                    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '-') {
                        i += 1;
                    }
                    continue;
                }
                if chars.get(i + 1) == Some(&'(') {
                    // Process substitution `>(cmd)` / `<(cmd)`.
                    let (inner, end) = take_balanced(&chars, i + 2);
                    segs.extend(split_segments(&inner));
                    i = end;
                    continue;
                }
                pending_redirect = is_out;
                if !is_out {
                    // Input redirection target is just consumed as a (harmless) word.
                    pending_redirect = false;
                    // Swallow the next word.
                    i += 1;
                    while i < chars.len() && chars[i] == ' ' {
                        i += 1;
                    }
                    while i < chars.len() && !" \t;|&()".contains(chars[i]) {
                        i += 1;
                    }
                    continue;
                }
            }
            _ => {
                in_word = true;
                word.push(c);
            }
        }
        i += 1;
    }
    finish_word(&mut word, &mut in_word, &mut pending_redirect, &mut cur);
    if !cur.words.is_empty() || !cur.redirects.is_empty() {
        segs.push(cur);
    }
    segs
}

/// Returns the content up to the matching `)` and the index after it.
fn take_balanced(chars: &[char], start: usize) -> (String, usize) {
    let mut depth = 1;
    let mut i = start;
    let mut out = String::new();
    while i < chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (out, i + 1);
                }
            }
            _ => {}
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, i)
}

fn is_block_device(target: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^/dev/(sd|hd|vd|xvd|nvme|mmcblk|disk|md|dm-)").unwrap())
        .is_match(target)
}

fn classify_segment(seg: &Segment) -> RiskClass {
    let mut risk = RiskClass::Read;
    for target in &seg.redirects {
        if is_block_device(target) {
            return RiskClass::Destructive;
        }
        if target != "/dev/null" && target != "/dev/stdout" && target != "/dev/stderr" {
            risk = RiskClass::Write;
        }
    }
    max_risk(risk, classify_words(&seg.words))
}

const READ_ONLY: &[&str] = &[
    "ls", "ll", "la", "dir", "cat", "tac", "less", "more", "head", "tail", "grep", "egrep",
    "fgrep", "rg", "ag", "zgrep", "zcat", "df", "du", "free", "uptime", "ps", "pgrep", "top",
    "htop", "atop", "whoami", "id", "groups", "hostname", "uname", "pwd", "echo", "printf",
    "date", "cal", "which", "whereis", "type", "printenv", "stat", "file", "wc", "uniq", "cut",
    "tr", "column", "nl", "diff", "cmp", "comm", "jq", "yq", "lsblk", "blkid", "lscpu", "lsmem",
    "lspci", "lsusb", "lsmod", "lsof", "ss", "netstat", "ping", "ping6", "traceroute",
    "tracepath", "mtr", "dig", "nslookup", "host", "whois", "tree", "basename", "dirname",
    "realpath", "readlink", "md5sum", "sha1sum", "sha256sum", "sha512sum", "cksum", "test", "[",
    "[[", "true", "false", "sleep", "man", "help", "history", "last", "lastlog", "w", "who",
    "getent", "nproc", "arch", "vmstat", "iostat", "mpstat", "sar", "pidstat", "nvidia-smi",
    "cd", "pushd", "popd", "hexdump", "xxd", "od", "strings", "seq", "yes", "locate", "dmidecode",
    "ldd", "timedatectl", "hostnamectl", "localectl", "loginctl", "uptime", "env", "export",
    "alias", "set", "watch", "column", "fold", "fmt", "expand", "base64", "openssl", "nc",
    "telnet", "dpkg-query", "systemd-analyze", "zless", "bat", "exa", "eza", "fd", "ncdu",
];

fn basename(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or(w)
}

fn has_flag(args: &[String], short: char, long: &str) -> bool {
    args.iter().any(|a| {
        if let Some(l) = a.strip_prefix("--") {
            !long.is_empty() && (l == long || l.starts_with(&format!("{long}=")))
        } else if let Some(s) = a.strip_prefix('-') {
            !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric()) && s.contains(short)
        } else {
            false
        }
    })
}

fn classify_words(words: &[String]) -> RiskClass {
    // Skip leading environment assignments.
    let mut idx = 0;
    while idx < words.len() && is_assignment(&words[idx]) {
        idx += 1;
    }
    if idx >= words.len() {
        // Pure assignments (`FOO=bar`) only affect the shell itself.
        return if idx == 0 {
            RiskClass::Read
        } else {
            RiskClass::Write
        };
    }
    let cmd = basename(&words[idx]);
    let args = &words[idx + 1..];
    let sub = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .unwrap_or("");

    match cmd {
        "sudo" | "doas" => {
            // Skip sudo options (and their values for the common ones).
            let mut j = 0;
            while j < args.len() && args[j].starts_with('-') {
                if matches!(args[j].as_str(), "-u" | "-g" | "-C" | "-h" | "-p" | "-U") {
                    j += 1;
                }
                j += 1;
            }
            return max_risk(RiskClass::Write, classify_words(&args[j.min(args.len())..]));
        }
        "time" | "nice" | "nohup" | "command" | "builtin" | "exec" | "stdbuf" | "ionice"
        | "unbuffer" => {
            let rest: Vec<String> = args
                .iter()
                .skip_while(|a| a.starts_with('-') || a.parse::<i64>().is_ok())
                .cloned()
                .collect();
            return classify_words(&rest);
        }
        "timeout" => {
            let rest: Vec<String> = args
                .iter()
                .skip_while(|a| a.starts_with('-'))
                .skip(1)
                .cloned()
                .collect();
            return classify_words(&rest);
        }
        "xargs" => {
            let rest: Vec<String> = args
                .iter()
                .skip_while(|a| a.starts_with('-'))
                .cloned()
                .collect();
            return if rest.is_empty() {
                RiskClass::Read
            } else {
                classify_words(&rest)
            };
        }
        "env" if !args.is_empty() => {
            let rest: Vec<String> = args
                .iter()
                .skip_while(|a| a.starts_with('-') || is_assignment(a))
                .cloned()
                .collect();
            return if rest.is_empty() {
                RiskClass::Read
            } else {
                classify_words(&rest)
            };
        }
        "rm" => {
            let recursive = has_flag(args, 'r', "recursive") || has_flag(args, 'R', "");
            let force = has_flag(args, 'f', "force");
            return if recursive && force {
                RiskClass::Destructive
            } else {
                RiskClass::Write
            };
        }
        "shred" | "wipefs" | "fdisk" | "sfdisk" | "gdisk" | "sgdisk" | "parted" | "shutdown"
        | "reboot" | "halt" | "poweroff" | "mkswap" | "lvremove" | "vgremove" | "pvremove"
        | "userdel" | "deluser" | "groupdel" => return RiskClass::Destructive,
        c if c.starts_with("mkfs") => return RiskClass::Destructive,
        "init" | "telinit" if matches!(sub, "0" | "6") => return RiskClass::Destructive,
        "dd" => {
            return if args.iter().any(|a| a.starts_with("of=")) {
                RiskClass::Destructive
            } else {
                RiskClass::Read
            };
        }
        "truncate" => return RiskClass::Destructive,
        "chmod" | "chown" | "chgrp" => {
            let recursive = has_flag(args, 'R', "recursive");
            let root = args.iter().any(|a| a == "/" || a == "/*");
            return if recursive && root {
                RiskClass::Destructive
            } else {
                RiskClass::Write
            };
        }
        "kill" => {
            let nine = args
                .iter()
                .any(|a| matches!(a.as_str(), "-9" | "-KILL" | "-SIGKILL" | "-s"));
            let pid1 = args.iter().any(|a| a == "1" || a == "-1");
            return if nine && pid1 {
                RiskClass::Destructive
            } else {
                RiskClass::Write
            };
        }
        "iptables" | "ip6tables" => {
            return if has_flag(args, 'F', "flush") || has_flag(args, 'X', "delete-chain") {
                RiskClass::Destructive
            } else if has_flag(args, 'L', "list") || has_flag(args, 'S', "list-rules") {
                RiskClass::Read
            } else {
                RiskClass::Write
            };
        }
        "nft" => {
            return if args.iter().any(|a| a == "flush") {
                RiskClass::Destructive
            } else if sub == "list" {
                RiskClass::Read
            } else {
                RiskClass::Write
            };
        }
        "crontab" => {
            return if has_flag(args, 'r', "") {
                RiskClass::Destructive
            } else if has_flag(args, 'l', "") {
                RiskClass::Read
            } else {
                RiskClass::Write
            };
        }
        "systemctl" => {
            return match sub {
                "status" | "show" | "list-units" | "list-unit-files" | "list-timers"
                | "list-sockets" | "list-dependencies" | "is-active" | "is-enabled"
                | "is-failed" | "cat" | "get-default" | "" => RiskClass::Read,
                "reboot" | "poweroff" | "halt" | "kexec" | "emergency" | "rescue" => {
                    RiskClass::Destructive
                }
                _ => RiskClass::Write,
            };
        }
        "service" => {
            return if args.get(1).map(|s| s.as_str()) == Some("status")
                || has_flag(args, 'l', "status-all")
            {
                RiskClass::Read
            } else {
                RiskClass::Write
            };
        }
        "journalctl" => {
            return if args
                .iter()
                .any(|a| a.starts_with("--vacuum") || a == "--rotate" || a == "--flush")
            {
                RiskClass::Write
            } else {
                RiskClass::Read
            };
        }
        "dmesg" => {
            return if has_flag(args, 'c', "read-clear") || has_flag(args, 'C', "clear") {
                RiskClass::Write
            } else {
                RiskClass::Read
            };
        }
        "find" => {
            return if args.iter().any(|a| a == "-delete") {
                RiskClass::Destructive
            } else if args
                .iter()
                .any(|a| matches!(a.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir"))
            {
                // Classify the exec'd command.
                let pos = args
                    .iter()
                    .position(|a| a.starts_with("-exec") || a.starts_with("-ok"))
                    .unwrap_or(0);
                let inner: Vec<String> = args[pos + 1..]
                    .iter()
                    .take_while(|a| *a != ";" && *a != "+" && *a != "\\;")
                    .cloned()
                    .collect();
                max_risk(RiskClass::Write, classify_words(&inner))
            } else if args.iter().any(|a| a.starts_with("-fprint")) {
                RiskClass::Write
            } else {
                RiskClass::Read
            };
        }
        "sed" => {
            return if has_flag(args, 'i', "in-place") {
                RiskClass::Write
            } else {
                RiskClass::Read
            };
        }
        "awk" | "gawk" | "mawk" => {
            return if args.iter().any(|a| a.contains("system(") || a.contains("print >")) {
                RiskClass::Write
            } else {
                RiskClass::Read
            };
        }
        "sort" => {
            return if has_flag(args, 'o', "output") {
                RiskClass::Write
            } else {
                RiskClass::Read
            };
        }
        "curl" => {
            let writes = args.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "-d" | "--data" | "--data-raw" | "--data-binary" | "-F" | "--form" | "-T"
                        | "--upload-file" | "-o" | "--output" | "-O" | "--remote-name"
                ) || a.starts_with("--data")
            }) || args.windows(2).any(|w| {
                (w[0] == "-X" || w[0] == "--request") && !w[1].eq_ignore_ascii_case("GET")
            });
            return if writes {
                RiskClass::Write
            } else {
                RiskClass::Read
            };
        }
        "ip" => {
            return if args.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "add" | "del" | "delete" | "set" | "flush" | "change" | "replace" | "append"
                )
            }) {
                RiskClass::Write
            } else {
                RiskClass::Read
            };
        }
        "sysctl" => {
            return if has_flag(args, 'w', "write") || args.iter().any(|a| a.contains('=')) {
                RiskClass::Write
            } else {
                RiskClass::Read
            };
        }
        "mount" | "umount" => {
            return if args.is_empty() || (cmd == "mount" && has_flag(args, 'l', "")) {
                RiskClass::Read
            } else {
                RiskClass::Write
            };
        }
        "git" => return classify_git(args),
        "docker" | "podman" | "nerdctl" => return classify_docker(args),
        "kubectl" | "oc" | "k" => return classify_kubectl(args),
        "helm" => {
            return match sub {
                "list" | "ls" | "status" | "get" | "history" | "show" | "search" | "version"
                | "template" | "lint" => RiskClass::Read,
                "uninstall" | "delete" | "del" => RiskClass::Destructive,
                _ => RiskClass::Write,
            };
        }
        "apt" | "apt-get" | "yum" | "dnf" | "zypper" | "apk" | "pacman" | "brew" => {
            return match sub {
                "list" | "search" | "show" | "info" | "policy" | "madison" | "list-installed"
                | "repolist" => RiskClass::Read,
                _ if cmd == "pacman" && has_flag(args, 'Q', "query") => RiskClass::Read,
                _ => RiskClass::Write,
            };
        }
        "dpkg" => {
            return if has_flag(args, 'l', "list")
                || has_flag(args, 'L', "listfiles")
                || has_flag(args, 's', "status")
            {
                RiskClass::Read
            } else {
                RiskClass::Write
            };
        }
        "rpm" => {
            return if has_flag(args, 'q', "query") {
                RiskClass::Read
            } else {
                RiskClass::Write
            };
        }
        "pip" | "pip3" | "npm" | "pnpm" | "yarn" | "cargo" | "gem" => {
            return match sub {
                "list" | "ls" | "show" | "freeze" | "view" | "outdated" | "search" | "--version"
                | "version" => RiskClass::Read,
                _ => RiskClass::Write,
            };
        }
        "zfs" | "zpool" => {
            return match sub {
                "destroy" => RiskClass::Destructive,
                "list" | "get" | "status" | "iostat" => RiskClass::Read,
                _ => RiskClass::Write,
            };
        }
        "mysql" | "psql" | "sqlite3" | "redis-cli" | "mongo" | "mongosh" => {
            // SQL destructive statements are caught by the global regex; anything else may write.
            return if cmd == "redis-cli"
                && args
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case("flushall") || a.eq_ignore_ascii_case("flushdb"))
            {
                RiskClass::Destructive
            } else {
                RiskClass::Write
            };
        }
        "tee" => return RiskClass::Write,
        _ => {}
    }
    if READ_ONLY.contains(&cmd) {
        RiskClass::Read
    } else {
        RiskClass::Write
    }
}

fn is_assignment(w: &str) -> bool {
    match w.find('=') {
        Some(p) if p > 0 => w[..p]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'),
        _ => false,
    }
}

fn classify_git(args: &[String]) -> RiskClass {
    // Skip global options like `-C dir`, `-c k=v`, `--no-pager`.
    let mut j = 0;
    while j < args.len() && args[j].starts_with('-') {
        if args[j] == "-C" || args[j] == "-c" {
            j += 1;
        }
        j += 1;
    }
    let Some(sub) = args.get(j) else {
        return RiskClass::Read;
    };
    let rest = &args[j + 1..];
    match sub.as_str() {
        "status" | "log" | "diff" | "show" | "rev-parse" | "describe" | "blame" | "ls-files"
        | "ls-tree" | "ls-remote" | "shortlog" | "reflog" | "grep" | "cat-file" | "whatchanged"
        | "help" | "version" | "rev-list" | "for-each-ref" => RiskClass::Read,
        "branch" => {
            if has_flag(rest, 'D', "") {
                RiskClass::Destructive
            } else if rest.is_empty()
                || rest.iter().all(|a| {
                    matches!(
                        a.as_str(),
                        "-a" | "-r" | "-v" | "-vv" | "--all" | "--list" | "--show-current"
                    )
                })
            {
                RiskClass::Read
            } else {
                RiskClass::Write
            }
        }
        "remote" | "tag" | "stash" if rest.is_empty() || rest[0] == "-v" || rest[0] == "list" => {
            RiskClass::Read
        }
        "config" if rest.iter().any(|a| a == "--get" || a == "--list" || a == "-l") => {
            RiskClass::Read
        }
        "reset" if rest.iter().any(|a| a == "--hard") => RiskClass::Destructive,
        "clean" if has_flag(rest, 'f', "force") => RiskClass::Destructive,
        "push"
            if rest
                .iter()
                .any(|a| a == "-f" || a == "--force" || a.starts_with("--force") || a.starts_with('+')) =>
        {
            RiskClass::Destructive
        }
        _ => RiskClass::Write,
    }
}

fn classify_docker(args: &[String]) -> RiskClass {
    let words: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();
    let force = has_flag(args, 'f', "force");
    match words.as_slice() {
        ["ps", ..] | ["images", ..] | ["logs", ..] | ["inspect", ..] | ["stats", ..]
        | ["version", ..] | ["info", ..] | ["top", ..] | ["port", ..] | ["diff", ..]
        | ["history", ..] | ["search", ..] | [] => RiskClass::Read,
        [_, "ls", ..] | [_, "list", ..] | [_, "inspect", ..] | [_, "ps", ..] | [_, "logs", ..] => {
            RiskClass::Read
        }
        ["compose", "config", ..] => RiskClass::Read,
        ["system", "prune", ..] | ["volume", "prune", ..] | ["volume", "rm", ..]
        | ["image", "prune", ..] | ["container", "prune", ..] | ["network", "prune", ..]
        | ["compose", "down", ..]
            if force || args.iter().any(|a| a == "-v" || a == "--volumes" || a == "-a") =>
        {
            RiskClass::Destructive
        }
        ["system", "prune", ..] | ["volume", "rm", ..] | ["volume", "prune", ..] => {
            RiskClass::Destructive
        }
        ["rm", ..] | ["rmi", ..] if force => RiskClass::Destructive,
        _ => RiskClass::Write,
    }
}

fn classify_kubectl(args: &[String]) -> RiskClass {
    let mut j = 0;
    while j < args.len() && args[j].starts_with('-') {
        if matches!(args[j].as_str(), "-n" | "--namespace" | "--context" | "--kubeconfig") {
            j += 1;
        }
        j += 1;
    }
    let sub = args.get(j).map(|s| s.as_str()).unwrap_or("");
    match sub {
        "get" | "describe" | "logs" | "top" | "version" | "explain" | "api-resources"
        | "api-versions" | "cluster-info" | "events" | "auth" | "diff" | "" => RiskClass::Read,
        "config" if matches!(args.get(j + 1).map(|s| s.as_str()), Some("view" | "get-contexts" | "current-context")) => {
            RiskClass::Read
        }
        "delete" | "drain" => RiskClass::Destructive,
        _ => RiskClass::Write,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RiskClass::*;

    #[test]
    fn classification_table() {
        let cases: &[(&str, RiskClass)] = &[
            ("ls -la /var/log", Read),
            ("cat /etc/os-release", Read),
            ("grep -r error /var/log | head -n 20", Read),
            ("df -h", Read),
            ("ps aux | grep nginx", Read),
            ("systemctl status nginx", Read),
            ("journalctl -u nginx --since today", Read),
            ("docker ps -a", Read),
            ("kubectl get pods -n kube-system", Read),
            ("kubectl -n prod describe pod x", Read),
            ("git status", Read),
            ("git log --oneline -5", Read),
            ("free -m && uptime", Read),
            ("echo hello", Read),
            ("cd /tmp; ls", Read),
            ("tail -f /var/log/syslog", Read),
            ("find / -name '*.conf'", Read),
            ("du -sh * 2>/dev/null | sort -h", Read),
            ("curl -s https://example.com", Read),
            ("ip addr show", Read),
            ("docker compose ls", Read),
            ("echo $(whoami)", Read),
            ("sed -n 1,10p file", Read),
            ("FOO=1 printenv FOO", Read),
            ("echo hi > out.txt", Write),
            ("echo hi >> /etc/motd", Write),
            ("touch /tmp/x", Write),
            ("mkdir -p /opt/app", Write),
            ("rm file.txt", Write),
            ("systemctl restart nginx", Write),
            ("apt-get install -y nginx", Write),
            ("sudo ls /root", Write),
            ("sudo -u postgres psql -c 'select 1'", Write),
            ("ls | xargs touch", Write),
            ("sed -i s/a/b/ file", Write),
            ("curl -X POST https://api", Write),
            ("git commit -m 'x'", Write),
            ("docker run -d nginx", Write),
            ("kubectl apply -f deploy.yaml", Write),
            ("ls && make install", Write),
            ("cat foo | tee bar", Write),
            ("echo $(touch x)", Write),
            ("ls 2>&1", Read),
            ("ls > /dev/null 2>&1", Read),
            ("rm -rf /var/www", Destructive),
            ("rm -fr build", Destructive),
            ("rm -r -f build", Destructive),
            ("sudo rm --recursive --force /opt", Destructive),
            ("mkfs.ext4 /dev/sdb1", Destructive),
            ("dd if=/dev/zero of=/dev/sda bs=1M", Destructive),
            ("shutdown -h now", Destructive),
            ("sudo reboot", Destructive),
            ("mysql -e 'DROP DATABASE prod'", Destructive),
            ("psql -c \"drop table users\"", Destructive),
            ("truncate -s 0 /var/log/syslog", Destructive),
            ("echo x > /dev/sda", Destructive),
            ("chmod -R 777 /", Destructive),
            ("kill -9 1", Destructive),
            ("iptables -F", Destructive),
            ("ls; rm -rf /tmp/x", Destructive),
            ("echo ok && (cd / && rm -rf *)", Destructive),
            ("git reset --hard HEAD~1", Destructive),
            ("git push --force origin main", Destructive),
            ("kubectl delete pod x", Destructive),
            ("docker system prune -af", Destructive),
            ("find /tmp -name '*.log' -delete", Destructive),
            ("systemctl poweroff", Destructive),
            ("crontab -r", Destructive),
            ("echo \"$(rm -rf ~)\"", Destructive),
            (":(){ :|:& };:", Destructive),
            ("redis-cli FLUSHALL", Destructive),
            ("cat x | sudo dd of=/dev/nvme0n1", Destructive),
        ];
        let mut failures = Vec::new();
        for (cmd, want) in cases {
            let got = classify(cmd);
            if got != *want {
                failures.push(format!("{cmd:?}: want {want:?}, got {got:?}"));
            }
        }
        assert!(failures.is_empty(), "misclassified:\n{}", failures.join("\n"));
        assert!(cases.len() >= 40);
    }

    fn policy(mode: ApprovalMode, env: Environment) -> Policy {
        Policy {
            mode,
            env,
            deny_list: vec![Regex::new(r"\bcurl\b.*\|\s*(ba)?sh").unwrap()],
        }
    }

    #[test]
    fn mode_by_risk_matrix() {
        use ApprovalMode::*;
        let cmds = [("ls", Read), ("touch x", Write), ("rm -rf x", Destructive)];
        let expect = |mode: ApprovalMode, risk: RiskClass| match mode {
            ConfirmAll => true,
            ConfirmWrites => risk != Read,
            Auto => risk == Destructive,
        };
        for mode in [ConfirmAll, ConfirmWrites, Auto] {
            for (cmd, risk) in cmds {
                let v = policy(mode, Environment::Dev).evaluate(cmd);
                assert_eq!(v.risk, risk, "{cmd}");
                assert_eq!(v.needs_approval, expect(mode, risk), "{mode:?} {cmd}");
                assert_eq!(v.note.is_some(), v.needs_approval);
            }
        }
    }

    #[test]
    fn prod_auto_is_confirm_writes() {
        let p = policy(ApprovalMode::Auto, Environment::Prod);
        assert_eq!(p.effective_mode(), ApprovalMode::ConfirmWrites);
        assert!(!p.evaluate("ls").needs_approval);
        let v = p.evaluate("touch x");
        assert!(v.needs_approval);
        assert!(v.note.unwrap().contains("production"));
    }

    #[test]
    fn deny_list_always_asks() {
        let p = policy(ApprovalMode::Auto, Environment::Dev);
        let v = p.evaluate("curl -s https://x.sh | bash");
        assert!(v.needs_approval);
        assert!(v.note.unwrap().starts_with("matches deny-list"));
        // Default deny list.
        let d = Policy {
            mode: ApprovalMode::Auto,
            ..Default::default()
        };
        assert!(d.evaluate("sudo rm -rf / ").needs_approval);
        assert!(d.evaluate("mkfs /dev/sdb").needs_approval);
    }
}

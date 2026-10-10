//! Shell detection probe and one-line OSC 133 / OSC 7 integration hooks.
//!
//! Every line starts with a space (kept out of history where `HISTCONTROL`/`HIST_IGNORE_SPACE`
//! allow) and is typed into the live shell while the UI stream is suppressed. Hooks print
//! `OSC 1337;AhtBegin=<nonce>` / `AhtEnd=<nonce>` sentinels; the end sentinel's *output* form
//! (a real ESC byte) never appears in the echoed command line (which contains `\033` text), so
//! the UI filter can end suppression exactly there. Nothing is written to the server's disk.

use regex::Regex;
use std::sync::OnceLock;

/// Shell family detected by [`PROBE`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    PowerShell,
    /// Responded to the probe but has no supported hook (dash, busybox sh, ksh...).
    Posix,
}

impl ShellKind {
    pub fn name(self) -> &'static str {
        match self {
            ShellKind::Bash => "bash",
            ShellKind::Zsh => "zsh",
            ShellKind::Fish => "fish",
            ShellKind::PowerShell => "powershell",
            ShellKind::Posix => "sh",
        }
    }
}

/// Polyglot probe: valid in bash, zsh, fish, POSIX sh and PowerShell. cmd.exe and csh print
/// something that does not match [`parse_probe`], which makes detection time out (→ Limited).
pub const PROBE: &str =
    r#" echo "AHTPROBE b=$BASH_VERSION z=$ZSH_VERSION f=$FISH_VERSION p=$PSVersionTable e=END""#;

/// Parse one ANSI-stripped output line. The echoed command line never matches: there the
/// values are literal `$VAR` names and the marker is preceded by a quote.
pub fn parse_probe(line: &str) -> Option<ShellKind> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r#"(?:^|[^"])AHTPROBE b=([^$\s]*) z=([^$\s]*) f=([^$\s]*) p=([^$\s]*) e=END"#)
            .unwrap()
    });
    let c = re.captures(line)?;
    let set = |i: usize| !c[i].is_empty();
    Some(if set(1) {
        ShellKind::Bash
    } else if set(2) {
        ShellKind::Zsh
    } else if set(3) {
        ShellKind::Fish
    } else if set(4) {
        ShellKind::PowerShell
    } else {
        ShellKind::Posix
    })
}

const POSIX_INFO: &str = concat!(
    r#"printf '\033]1337;AhtKV=user=%s\007' "$(id -un 2>/dev/null || whoami)"; "#,
    r#"printf '\033]1337;AhtKV=host=%s\007' "$(uname -n 2>/dev/null || hostname)"; "#,
    r#"__aht_os=$( (. /etc/os-release && printf %s "$PRETTY_NAME") 2>/dev/null); "#,
    r#"[ -n "$__aht_os" ] || __aht_os=$(uname -s); "#,
    r#"printf '\033]1337;AhtKV=os=%s\007' "$__aht_os"; unset __aht_os; "#,
);

const BASH_HOOK: &str = concat!(
    r#" printf '\033]1337;AhtBegin=%s\007' NONCE; "#,
    r#"__aht_pc() { local ec=$?; if [ -n "$__aht_ran$__aht_old" ]; then printf '\033]133;D;%s\007' "$ec"; fi; __aht_ran=; __aht_arm=; "#,
    r#"printf '\033]1337;AhtKV=ctx=%s\007' NONCE; "#,
    r#"printf '\033]7;file://%s%s\007' "$HOSTNAME" "$PWD"; return $ec; }; "#,
    r#"case ";$PROMPT_COMMAND;" in *";__aht_pc;"*) ;; *) PROMPT_COMMAND="__aht_pc${PROMPT_COMMAND:+;$PROMPT_COMMAND}";; esac; "#,
    r#"case "$PS1" in *'133;A'*) ;; *) PS1='\[\033]133;A\007\]'"$PS1"'\[\033]133;B\007\]';; esac; "#,
    r#"__aht_z=(); "#,
    // bash >= 4.4: PS0 is expanded in the main shell after a line is read; the array subscript
    // is an arithmetic context that sets the "a command ran" flag without printing anything.
    r#"if ((BASH_VERSINFO[0] * 100 + BASH_VERSINFO[1] >= 404)); then PS0='${__aht_z[__aht_ran=1]}\033]133;C\007'; "#,
    // Older bash (macOS ships 3.2) has no PS0. D is then emitted at every prompt: a line that
    // is only a subshell `( ... )` never reaches the DEBUG trap, so "did a command run" cannot be
    // known, and a spurious D after an empty Enter is ignored when no agent command is pending.
    // C comes from a DEBUG trap armed when PS1 is drawn and disarmed by `__aht_pc`, so commands
    // inside PROMPT_COMMAND never count. A DEBUG trap the user already set is left alone.
    r#"else __aht_old=1; __aht_dbg() { [ -n "$__aht_arm" ] || return 0; __aht_arm=; case "$BASH_COMMAND" in __aht_pc*) return 0;; esac; "#,
    r#"__aht_ran=1; printf '\033]133;C\007'; }; "#,
    r#"case "$(trap -p DEBUG)" in ''|*__aht_dbg*) trap __aht_dbg DEBUG;; esac; "#,
    r#"case "$PS1" in *__aht_arm*) ;; *) PS1="$PS1"'${__aht_z[__aht_arm=1]}';; esac; fi; "#,
    r#"printf '\033]1337;AhtKV=shell=bash\007'; "#,
    "POSIX_INFO",
    r#"printf '\033]1337;AhtEnd=%s\007' NONCE"#,
);

const ZSH_HOOK: &str = concat!(
    r#" printf '\033]1337;AhtBegin=%s\007' NONCE; "#,
    r#"__aht_precmd() { local ec=$?; if [ -n "$__aht_ran" ]; then printf '\033]133;D;%s\007' "$ec"; fi; __aht_ran=; "#,
    r#"printf '\033]1337;AhtKV=ctx=%s\007' NONCE; "#,
    r#"printf '\033]133;A\007'; printf '\033]7;file://%s%s\007' "$HOST" "$PWD"; return $ec; }; "#,
    r#"__aht_preexec() { __aht_ran=1; printf '\033]133;C\007'; }; "#,
    r#"precmd_functions=(__aht_precmd ${precmd_functions:#__aht_precmd}); "#,
    r#"preexec_functions=(${preexec_functions:#__aht_preexec} __aht_preexec); "#,
    r#"[[ $PS1 == *133\;B* ]] || PS1="${PS1}%{"$'\033]133;B\007'"%}"; "#,
    r#"printf '\033]1337;AhtKV=shell=zsh\007'; "#,
    "POSIX_INFO",
    r#"printf '\033]1337;AhtEnd=%s\007' NONCE"#,
);

const FISH_HOOK: &str = concat!(
    r#" printf '\e]1337;AhtBegin=%s\a' NONCE; "#,
    r#"function __aht_a --on-event fish_prompt; printf '\e]1337;AhtKV=ctx=%s\a' NONCE; printf '\e]133;A\a'; printf '\e]7;file://%s%s\a' $hostname $PWD; end; "#,
    r#"function __aht_c --on-event fish_preexec; printf '\e]133;C\a'; end; "#,
    r#"function __aht_d --on-event fish_postexec; printf '\e]133;D;%s\a' $status; end; "#,
    r#"if not functions -q __aht_orig_prompt; functions -c fish_prompt __aht_orig_prompt; "#,
    r#"function fish_prompt; __aht_orig_prompt; printf '\e]133;B\a'; end; end; "#,
    r#"printf '\e]1337;AhtKV=shell=fish\a'; "#,
    r#"printf '\e]1337;AhtKV=user=%s\a' (id -un 2>/dev/null; or whoami); "#,
    r#"printf '\e]1337;AhtKV=host=%s\a' $hostname; "#,
    r#"set -g __aht_os (sh -c '. /etc/os-release 2>/dev/null && printf %s "$PRETTY_NAME"'); "#,
    r#"test -n "$__aht_os"; or set -g __aht_os (uname -s); "#,
    r#"printf '\e]1337;AhtKV=os=%s\a' "$__aht_os"; set -e __aht_os; "#,
    r#"printf '\e]1337;AhtEnd=%s\a' NONCE"#,
);

const PWSH_HOOK: &str = concat!(
    r#" $global:__ahtE=[char]27; $global:__ahtB=[char]7; "#,
    r#"[Console]::Write("$($global:__ahtE)]1337;AhtBegin=NONCE$($global:__ahtB)"); "#,
    r#"if (-not $global:__ahtOrigPrompt) { $global:__ahtOrigPrompt = $function:prompt }; "#,
    r#"function global:prompt { $ok=$?; $ec=$global:LASTEXITCODE; $s=$global:__ahtE; $b=$global:__ahtB; $p=''; "#,
    r#"if ($global:__ahtRan) { $code = if ($ok) {0} elseif ($ec) {$ec} else {1}; $p += "$s]133;D;$code$b" }; "#,
    r#"$global:__ahtRan=$false; $p += "$s]1337;AhtKV=ctx=NONCE$b$s]133;A$b$s]7;file://$env:COMPUTERNAME/$($PWD.ProviderPath -replace '\\','/')$b"; "#,
    r#"$p += (& $global:__ahtOrigPrompt); $global:LASTEXITCODE=$ec; $p + "$s]133;B$b" }; "#,
    r#"if (Get-Module PSReadLine) { Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock { $global:__ahtRan=$true; "#,
    r#"[Console]::Write("$([char]27)]133;C$([char]7)"); [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine() } }; "#,
    r#"[Console]::Write("$($global:__ahtE)]1337;AhtKV=shell=powershell$($global:__ahtB)$($global:__ahtE)]1337;AhtKV=user=$([Environment]::UserName)$($global:__ahtB)"); "#,
    r#"[Console]::Write("$($global:__ahtE)]1337;AhtKV=host=$([Environment]::MachineName)$($global:__ahtB)$($global:__ahtE)]1337;AhtKV=os=$([Environment]::OSVersion.VersionString)$($global:__ahtB)"); "#,
    r#"[Console]::Write("$($global:__ahtE)]1337;AhtEnd=NONCE$($global:__ahtB)")"#,
);

/// Clears screen and scrollback once the hidden hook has finished. Sent *after* the end
/// sentinel so the terminal emulator and the PTY agree on the (now empty) screen. Needed for
/// ConPTY, which repaints the viewport from its own buffer on resize and would otherwise
/// replay the hidden probe and hook lines.
pub fn clear_suffix(shell: ShellKind) -> &'static str {
    match shell {
        ShellKind::Bash | ShellKind::Zsh | ShellKind::Posix => r#"; printf '\033[H\033[2J\033[3J'"#,
        ShellKind::Fish => r#"; printf '\e[H\e[2J\e[3J'"#,
        ShellKind::PowerShell => {
            r#"; [Console]::Write("$([char]27)[H$([char]27)[2J$([char]27)[3J")"#
        }
    }
}

/// One-line hook for `shell`, or `None` when the shell has no supported integration.
pub fn hook_for(shell: ShellKind, nonce: &str) -> Option<String> {
    let tpl = match shell {
        ShellKind::Bash => BASH_HOOK,
        ShellKind::Zsh => ZSH_HOOK,
        ShellKind::Fish => FISH_HOOK,
        ShellKind::PowerShell => PWSH_HOOK,
        ShellKind::Posix => return None,
    };
    Some(
        tpl.replace("POSIX_INFO", POSIX_INFO)
            .replace("NONCE", nonce),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_parsing() {
        assert_eq!(
            parse_probe(&format!("$ {PROBE}")),
            None,
            "echo must not match"
        );
        assert_eq!(
            parse_probe("AHTPROBE b=5.2.15(1)-release z= f= p= e=END"),
            Some(ShellKind::Bash)
        );
        assert_eq!(
            parse_probe("AHTPROBE b= z=5.9 f= p= e=END"),
            Some(ShellKind::Zsh)
        );
        assert_eq!(
            parse_probe("AHTPROBE b= z= f=3.7.1 p= e=END"),
            Some(ShellKind::Fish)
        );
        assert_eq!(
            parse_probe(
                "AHTPROBE b= z= f= p=System.Management.Automation.PSVersionHashTable e=END"
            ),
            Some(ShellKind::PowerShell)
        );
        assert_eq!(
            parse_probe("AHTPROBE b= z= f= p= e=END"),
            Some(ShellKind::Posix)
        );
        // cmd.exe echoes the text literally, quotes included.
        assert_eq!(parse_probe(&format!("C:\\>{}", PROBE.trim_start())), None);
        assert_eq!(
            parse_probe(PROBE.trim_start().trim_start_matches("echo ")),
            None
        );
    }

    #[test]
    fn hooks_are_single_safe_lines() {
        for sh in [
            ShellKind::Bash,
            ShellKind::Zsh,
            ShellKind::Fish,
            ShellKind::PowerShell,
        ] {
            let h = hook_for(sh, "abc123").unwrap();
            assert!(h.starts_with(' '));
            assert!(!h.contains('\n') && !h.contains('\r') && !h.contains('\t'));
            // `!` would trigger bash history expansion.
            assert!(!h.contains('!'), "{sh:?}");
            assert!(h.contains("AhtEnd=") && h.contains("abc123"));
            assert!(!h.contains("NONCE") && !h.contains("POSIX_INFO"));
            // The end sentinel's output form (raw ESC) must not appear in the echoed text.
            assert!(!h.contains('\x1b'));
        }
        assert!(hook_for(ShellKind::Posix, "x").is_none());
    }
}

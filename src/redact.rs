/// Scrub credential-like values from shell commands before they are stored.
///
/// The goal is best-effort: catch the most common patterns (passwords and tokens
/// passed as flag arguments) without false-positives on benign content.  We never
/// block recording — a command that looks fully opaque after redaction is still
/// recorded, just with the sensitive token replaced by `<REDACTED>`.
///
/// ## Coverage
///
/// | Pattern                            | Example                            | Outcome                       |
/// |------------------------------------|-------------------------------------|-------------------------------|
/// | flag + space + value               | `mysql -p s3cr3t`                  | `mysql -p <REDACTED>`         |
/// | flag=value                         | `curl --token=abc`                 | `curl --token=<REDACTED>`     |
/// | concatenated short flag+value      | `mysql -ps3cr3t`                   | `mysql -p<REDACTED>`          |
/// | env-var assignment                 | `API_KEY=xyz ./deploy.sh`          | `API_KEY=<REDACTED> ./deploy.sh` |
/// | command-substitution value         | `curl --token $(cat /tmp/tok)`     | `curl --token <REDACTED>`     |
/// | subshell-in-flag                   | `mysql -p$(echo s3cr3t)`           | `mysql -p<REDACTED>`          |
///
/// ## Known limitations
///
/// - `-p` is overloaded: `ssh -p 22` loses its port value. We err toward redaction.
/// - Indirect references (`TOKEN=$(vault read …); curl --token $TOKEN`) are two
///   separate commands; only the second can be caught if `$TOKEN` appears literally.
/// - Double-indirection (`eval …`, here-strings) is undecidable without actually
///   executing the shell — we don't attempt it.
///
/// ## Shell-side mitigation
///
/// - Prefix with a space + set `HISTIGNORE=" *"` to exclude from zsh/bash history
///   (tapeworm also skips those — or can be configured to).
/// - Use `read -rs SECRET` to avoid secrets appearing on the command line at all.

/// Flags whose next token (or concatenated suffix) is treated as a secret.
const SECRET_FLAGS: &[&str] = &[
    "--password",
    "--passwd",
    "--pass",
    "-p",
    "--secret",
    "--secret-key",
    "--secret-access-key",
    "--token",
    "--api-key",
    "--apikey",
    "--auth",
    "--auth-token",
    "--bearer",
    "--private-key",
    "--signing-key",
    "--access-key",
    "--access-key-id",
    "--client-secret",
];

/// Substrings that, when found in the KEY part of a KEY=VALUE token, mark the
/// VALUE for redaction.
const SECRET_ENV_SUBSTRINGS: &[&str] = &[
    "PASSWORD", "PASSWD", "SECRET", "TOKEN", "API_KEY", "APIKEY",
    "AUTH", "BEARER", "PRIVATE_KEY", "ACCESS_KEY", "SIGNING_KEY",
    "CLIENT_SECRET",
];

// Pre-computed set of 2-char short flags (e.g. "-p") for the concatenated check.
// We avoid a heap allocation at call time by checking the slice directly.
fn is_2char_secret_flag(s: &str) -> bool {
    SECRET_FLAGS.iter().any(|f| f.len() == 2 && *f == s)
}

pub fn redact_command(cmd: &str) -> String {
    let tokens = tokenize(cmd);
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut redact_next = false;

    'token: for token in &tokens {
        // ── The previous token was a credential flag: redact whatever this is ──
        if redact_next {
            out.push("<REDACTED>".to_string());
            redact_next = false;
            continue;
        }

        // ── flag=value  (long flags: --token=abc, --password=s3cr3t) ──
        if let Some((flag, _value)) = token.split_once('=') {
            let flag_lower = flag.to_ascii_lowercase();
            if SECRET_FLAGS.iter().any(|f| *f == flag_lower) {
                out.push(format!("{flag}=<REDACTED>"));
                continue 'token;
            }
            // ── env-var assignment: API_KEY=xyz or export MY_TOKEN=abc ──
            if !flag.starts_with('-') {
                let key_upper = flag.to_ascii_uppercase();
                if SECRET_ENV_SUBSTRINGS.iter().any(|s| key_upper.contains(s)) {
                    out.push(format!("{flag}=<REDACTED>"));
                    continue 'token;
                }
            }
        }

        // ── Concatenated short flag+value: -ps3cr3t  or  -p$(echo x) ──
        // A token like "-pFOO" where "-p" is a known 2-char secret flag.
        if token.len() > 2 && token.starts_with('-') && !token.starts_with("--") {
            let candidate_flag = &token[..2];
            if is_2char_secret_flag(&candidate_flag.to_ascii_lowercase()) {
                // Preserve the flag prefix, redact the rest.
                out.push(format!("{}<REDACTED>", candidate_flag));
                continue 'token;
            }
        }

        // ── Standalone flag: next token is the value ──
        let token_lower = token.to_ascii_lowercase();
        if SECRET_FLAGS.iter().any(|f| *f == token_lower) {
            out.push(token.clone());
            redact_next = true;
            continue 'token;
        }

        out.push(token.clone());
    }

    out.join(" ")
}

/// Tokenize a shell command into words.
///
/// Differences from a naive `split_whitespace`:
/// - Quoted spans (single or double) are kept together.
/// - `$(...)` subshells are kept as a single token — depth-tracking prevents
///   `$(cat /tmp/token)` from being split into `$(cat` and `/tmp/token)`.
/// - Backslash escapes are preserved verbatim.
///
/// We intentionally do *not* handle every POSIX shell construct (process
/// substitution `<(...)`, arithmetic `$((...))`, heredocs, etc.).  The goal is
/// to correctly tokenize the patterns that most commonly carry credentials.
fn tokenize(s: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut subshell_depth: usize = 0; // depth inside $(…) / (…)
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            // ── Backslash escape ──
            '\\' => {
                current.push(ch);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            // ── Single-quote toggle (only outside double-quotes and subshells) ──
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(ch);
            }
            // ── Double-quote toggle ──
            '"' if !in_single => {
                in_double = !in_double;
                current.push(ch);
            }
            // ── Dollar-paren: start of $(...) subshell ──
            '$' if !in_single => {
                current.push(ch);
                if chars.peek() == Some(&'(') {
                    subshell_depth += 1;
                    current.push(chars.next().unwrap()); // consume '('
                }
            }
            // ── Open paren outside quotes: bare subshell grouping (…) ──
            '(' if !in_single && !in_double => {
                subshell_depth += 1;
                current.push(ch);
            }
            // ── Close paren: exit subshell depth ──
            ')' if subshell_depth > 0 => {
                subshell_depth -= 1;
                current.push(ch);
            }
            // ── Whitespace: token boundary (only at top level) ──
            ' ' | '\t' if !in_single && !in_double && subshell_depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic flag patterns ──────────────────────────────────────────────────

    #[test]
    fn flag_space_value() {
        assert_eq!(
            redact_command("mysql -u root -p s3cr3t -h localhost"),
            "mysql -u root -p <REDACTED> -h localhost"
        );
    }

    #[test]
    fn flag_equals_value() {
        assert_eq!(
            redact_command("curl --token=abc123 https://api.example.com"),
            "curl --token=<REDACTED> https://api.example.com"
        );
    }

    #[test]
    fn env_var_assignment() {
        assert_eq!(
            redact_command("API_KEY=supersecret ./deploy.sh"),
            "API_KEY=<REDACTED> ./deploy.sh"
        );
    }

    #[test]
    fn export_assignment() {
        assert_eq!(
            redact_command("export AWS_SECRET_ACCESS_KEY=AKIA1234"),
            "export AWS_SECRET_ACCESS_KEY=<REDACTED>"
        );
    }

    #[test]
    fn benign_commands_unchanged() {
        let cmd = "cargo build --release";
        assert_eq!(redact_command(cmd), cmd);
    }

    #[test]
    fn nothing_after_flag() {
        // Flag at end of command — no crash, flag preserved as-is
        let out = redact_command("cmd --password");
        assert_eq!(out, "cmd --password");
    }

    // ── Concatenated short flag+value ────────────────────────────────────────

    #[test]
    fn concatenated_short_flag_plain_value() {
        // `mysql -ps3cr3t` — no space between flag and secret
        assert_eq!(
            redact_command("mysql -u root -ps3cr3t -h localhost"),
            "mysql -u root -p<REDACTED> -h localhost"
        );
    }

    #[test]
    fn concatenated_short_flag_subshell() {
        // `mysql -p$(echo s3cr3t)` — subshell glued directly to flag
        let out = redact_command("mysql -p$(echo s3cr3t) -h localhost");
        assert_eq!(out, "mysql -p<REDACTED> -h localhost");
    }

    // ── $(...) tokenizer — multi-word subshells stay atomic ─────────────────

    #[test]
    fn subshell_value_with_args() {
        // `$(cat /tmp/token)` must be a single token so the whole thing is redacted
        assert_eq!(
            redact_command("curl --token $(cat /tmp/token) https://api.example.com"),
            "curl --token <REDACTED> https://api.example.com"
        );
    }

    #[test]
    fn subshell_value_nested() {
        // Nested subshell: `$(vault kv get -field=token secret/myapp)`
        assert_eq!(
            redact_command("curl --token $(vault kv get -field=token secret/myapp) https://host"),
            "curl --token <REDACTED> https://host"
        );
    }

    #[test]
    fn subshell_value_quoted() {
        // Double-quoted subshell: --token "$(cat /tmp/token)"
        assert_eq!(
            redact_command(r#"curl --token "$(cat /tmp/token)" https://api.example.com"#),
            "curl --token <REDACTED> https://api.example.com"
        );
    }

    // ── -p overloading note ──────────────────────────────────────────────────

    #[test]
    fn p_flag_overload_ssh_port() {
        // Acknowledged false positive: ssh -p 22 loses port value.
        // We accept this — erring toward redaction.
        let out = redact_command("ssh -p 22 user@host");
        assert_eq!(out, "ssh -p <REDACTED> user@host");
    }
}

/// Scrub credential-like values from shell commands before they are stored.
///
/// The goal is best-effort: catch the most common patterns (passwords and tokens
/// passed as flag arguments) without false-positives on benign content.  We never
/// block recording — a command that looks fully opaque after redaction is still
/// recorded, just with the sensitive token replaced by `<REDACTED>`.
///
/// Shell-side mitigation (not tapeworm's responsibility, but worth documenting):
///   - Prefix a command with a space in zsh/bash (with HISTIGNORE=" *") to skip it
///   - Use `read -rs TOKEN` to avoid secrets appearing on the command line at all

/// Flags whose next token is treated as a secret value and replaced with `<REDACTED>`.
const SECRET_FLAGS: &[&str] = &[
    "--password",
    "--passwd",
    "--pass",
    "-p",           // mysql, psql, redis-cli, etc.
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

/// Patterns of the form `KEY=VALUE` where KEY suggests a secret.
/// The entire VALUE is replaced with `<REDACTED>`.
const SECRET_ENV_SUBSTRINGS: &[&str] = &[
    "PASSWORD", "PASSWD", "SECRET", "TOKEN", "API_KEY", "APIKEY",
    "AUTH", "BEARER", "PRIVATE_KEY", "ACCESS_KEY", "SIGNING_KEY",
    "CLIENT_SECRET",
];

pub fn redact_command(cmd: &str) -> String {
    let tokens = tokenize(cmd);
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut redact_next = false;

    for token in &tokens {
        if redact_next {
            out.push("<REDACTED>".to_string());
            redact_next = false;
            continue;
        }

        // Check flag=value style: --password=secret or --token=abc
        if let Some((flag, _value)) = token.split_once('=') {
            let flag_lower = flag.to_ascii_lowercase();
            if SECRET_FLAGS.iter().any(|f| *f == flag_lower) {
                out.push(format!("{flag}=<REDACTED>"));
                continue;
            }
        }

        // Check env-var assignment: PASSWORD=secret or MY_API_KEY=abc
        if !token.starts_with('-') {
            if let Some((key, _value)) = token.split_once('=') {
                let key_upper = key.to_ascii_uppercase();
                if SECRET_ENV_SUBSTRINGS.iter().any(|s| key_upper.contains(s)) {
                    out.push(format!("{key}=<REDACTED>"));
                    continue;
                }
            }
        }

        // Check stand-alone flag: next token is the secret
        let token_lower = token.to_ascii_lowercase();
        if SECRET_FLAGS.iter().any(|f| *f == token_lower) {
            out.push(token.clone());
            redact_next = true;
            continue;
        }

        out.push(token.clone());
    }

    out.join(" ")
}

/// Very simple tokenizer: split on whitespace, but keep quoted spans together.
/// We don't need a full shell parser here — the goal is just to avoid splitting
/// `--password "my secret"` into three pieces when reconstructing the command.
fn tokenize(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                current.push(ch);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(ch);
            }
            ' ' | '\t' if !in_single && !in_double => {
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
    fn no_false_positive_on_args() {
        // '-p 8080' for port should NOT be redacted — only the first hit
        // This is an acknowledged limitation; -p is overloaded.
        // We accept the false positive for safety.
        let out = redact_command("ssh -p 22 user@host");
        assert_eq!(out, "ssh -p <REDACTED> user@host");
    }

    #[test]
    fn benign_commands_unchanged() {
        let cmd = "cargo build --release";
        assert_eq!(redact_command(cmd), cmd);
    }

    #[test]
    fn nothing_after_flag() {
        // Flag at end of command — no crash
        let out = redact_command("cmd --password");
        assert_eq!(out, "cmd --password");
    }
}

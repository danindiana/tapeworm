/// A single step within a pipeline (one side of a `|`, `&&`, `||`, or `;`).
#[derive(Debug, Clone)]
pub struct PipelineStep {
    pub index: usize,
    /// The extracted tool name (argv[0], basename, wrappers stripped).
    pub tool: String,
    /// The raw text of this step, trimmed.
    pub raw: String,
    /// The operator that follows this step: `|`, `&&`, `||`, `;`, or `""` for the last step.
    pub connector: String,
}

/// Split a shell command string into pipeline steps.
///
/// Splits on `|`, `&&`, `||`, `;` at the top level only — respects single quotes,
/// double quotes, backslash escapes, `(` `)` subshell groupings, and `$(` expansions.
pub fn parse_pipeline(cmd: &str) -> Vec<PipelineStep> {
    let mut steps: Vec<(String, String)> = Vec::new(); // (raw, connector)
    let mut current = String::new();
    let mut chars = cmd.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut depth: usize = 0; // paren/subshell nesting depth

    while let Some(c) = chars.next() {
        match c {
            // --- Quote handling ---
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(c);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(c);
            }

            // --- Backslash escape (not in single quotes) ---
            '\\' if !in_single => {
                current.push(c);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }

            // --- $( subshell: increment depth (works in both normal and double-quote context) ---
            '$' if !in_single => {
                current.push(c);
                if chars.peek() == Some(&'(') {
                    current.push(chars.next().unwrap());
                    depth += 1;
                }
            }

            // --- Bare ( increments depth when not in any quote ---
            '(' if !in_single && !in_double => {
                depth += 1;
                current.push(c);
            }

            // --- ) decrements depth when not in single quotes.
            //     Using !in_single (not !in_double) so that ) inside "$(cmd)" closes correctly.
            ')' if !in_single => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }

            // --- Operators — only split at depth 0, outside all quotes ---
            '|' if !in_single && !in_double && depth == 0 => {
                if chars.peek() == Some(&'|') {
                    chars.next();
                    flush(&mut steps, &current, "||");
                } else {
                    flush(&mut steps, &current, "|");
                }
                current.clear();
            }

            '&' if !in_single && !in_double && depth == 0 => {
                if chars.peek() == Some(&'&') {
                    chars.next();
                    flush(&mut steps, &current, "&&");
                    current.clear();
                } else {
                    // Bare & (background job) — keep as part of step text
                    current.push(c);
                }
            }

            ';' if !in_single && !in_double && depth == 0 => {
                flush(&mut steps, &current, ";");
                current.clear();
            }

            _ => current.push(c),
        }
    }

    // Remaining text is the last step (connector = "")
    let raw = current.trim().to_string();
    if !raw.is_empty() {
        steps.push((raw, String::new()));
    }

    steps
        .into_iter()
        .enumerate()
        .map(|(i, (raw, connector))| {
            let tool = extract_tool(&raw);
            PipelineStep { index: i, tool, raw, connector }
        })
        .collect()
}

fn flush(steps: &mut Vec<(String, String)>, current: &str, connector: &str) {
    let raw = current.trim().to_string();
    if !raw.is_empty() {
        steps.push((raw, connector.to_string()));
    }
}

/// Wrapper commands that take only flags before the real command.
/// Flag-skipping is enabled after any of these are seen.
const WRAPPERS: &[&str] = &[
    // privilege / environment
    "sudo", "env", "doas",
    // scheduling / resource control (flag-only forms: nice -n N, ionice -c C -n N, chrt -f 99)
    "nice", "ionice", "chrt",
    // buffering / I/O control (flags like -oL)
    "stdbuf",
    // execution control
    "nohup", "time",
    // shell builtins and meta-wrappers
    "watch", "command", "builtin",
];

/// Wrappers that take exactly one positional (non-flag) argument before the command.
/// `skip_next` is set when these are encountered so the argument is consumed.
/// Example: `timeout 30 curl` — 30 is a bare positional, not a flag.
const WRAPPERS_POSITIONAL: &[&str] = &[
    "timeout",   // timeout DURATION CMD
];

/// Extract the tool name from a pipeline step string.
///
/// Strips leading env-var assignments (`FOO=bar`), known wrapper commands,
/// flags belonging to wrappers (`-u`, `--user`, etc.), and path prefixes.
///
/// Short flags of the form `-X` (exactly 2 chars) are assumed to consume the
/// following token as their argument (e.g. `sudo -u root cmd` → `cmd`).
/// Wrapper detection always takes precedence over flag-skipping so that
/// chained wrappers like `sudo timeout 60 rsync` are handled correctly.
pub fn extract_tool(step: &str) -> String {
    let mut skip_flags = false;
    let mut skip_next = false; // true when a short flag or positional-arg wrapper consumed a slot
    for tok in step.split_ascii_whitespace() {
        // Consume argument slot of a preceding short flag or positional wrapper arg
        if skip_next {
            skip_next = false;
            continue;
        }
        // Skip env-var assignments: FOO=bar, _VAR=x, etc.
        if looks_like_assignment(tok) {
            continue;
        }
        // Wrapper detection before flag-skipping: handles chained wrappers correctly.
        if WRAPPERS.contains(&tok) {
            skip_flags = true;
            continue;
        }
        // Wrappers with one bare positional argument (e.g. timeout DURATION)
        if WRAPPERS_POSITIONAL.contains(&tok) {
            skip_next = true;
            continue;
        }
        // Skip flags belonging to the preceding wrapper
        if skip_flags && tok.starts_with('-') {
            // Short flags (-X, 2 chars) typically take an argument — skip it too
            if tok.len() == 2 {
                skip_next = true;
            }
            continue;
        }
        let _ = skip_flags; // consumed above; reset is implicit via `return` below
        // Strip any path prefix (e.g. `/usr/bin/grep` → `grep`)
        return tok.rsplit('/').next().unwrap_or(tok).to_string();
    }
    String::new()
}

/// Returns true if `tok` looks like a shell env-var assignment: `NAME=value`.
/// The name part must be non-empty and consist only of alphanumerics and `_`.
fn looks_like_assignment(tok: &str) -> bool {
    if let Some(eq_pos) = tok.find('=') {
        eq_pos > 0 && tok[..eq_pos].chars().all(|c| c.is_alphanumeric() || c == '_')
    } else {
        false
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(cmd: &str) -> Vec<String> {
        parse_pipeline(cmd).into_iter().map(|s| s.tool).collect()
    }

    fn connectors(cmd: &str) -> Vec<String> {
        parse_pipeline(cmd).into_iter().map(|s| s.connector).collect()
    }

    #[test]
    fn simple_pipe() {
        assert_eq!(tools("grep foo bar.txt | sort | uniq -c"), vec!["grep", "sort", "uniq"]);
        assert_eq!(connectors("grep foo | sort"), vec!["|", ""]);
    }

    #[test]
    fn and_chain() {
        assert_eq!(tools("make && make install"), vec!["make", "make"]);
        assert_eq!(connectors("make && make install"), vec!["&&", ""]);
    }

    #[test]
    fn or_chain() {
        assert_eq!(tools("git pull || echo failed"), vec!["git", "echo"]);
        assert_eq!(connectors("git pull || echo failed"), vec!["||", ""]);
    }

    #[test]
    fn semicolon_sequence() {
        assert_eq!(tools("cd /tmp; ls -la"), vec!["cd", "ls"]);
    }

    #[test]
    fn single_command() {
        let steps = parse_pipeline("ls -la");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].tool, "ls");
        assert_eq!(steps[0].connector, "");
    }

    #[test]
    fn quoted_pipe_not_split() {
        // The | inside single quotes must not be treated as a pipe operator
        let steps = parse_pipeline("echo 'hello | world'");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].tool, "echo");
    }

    #[test]
    fn subshell_pipe_not_split() {
        // The | inside $(...) must not be treated as a pipeline split
        let steps = parse_pipeline("echo \"$(date | tr ' ' '_')\"");
        assert_eq!(steps.len(), 1);
    }

    #[test]
    fn grouped_subshell() {
        // (cmd1 && cmd2) | cmd3 — the && inside parens should not split
        let steps = parse_pipeline("(cd /tmp && ls) | grep foo");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].connector, "|");
        assert_eq!(steps[1].tool, "grep");
    }

    #[test]
    fn wrapper_stripping() {
        assert_eq!(tools("sudo apt install vim"), vec!["apt"]);
        assert_eq!(tools("sudo -u root rsync -av /src /dst"), vec!["rsync"]);
        assert_eq!(tools("FOO=bar BAZ=qux grep pattern file"), vec!["grep"]);
        assert_eq!(tools("env PATH=/usr/local/bin cargo build"), vec!["cargo"]);
        assert_eq!(tools("time nice -n 10 make -j4"), vec!["make"]);
    }

    #[test]
    fn wrapper_stripping_extended() {
        // stdbuf: line-buffer output of a command
        assert_eq!(tools("stdbuf -oL grep foo /var/log/syslog"), vec!["grep"]);
        // timeout: kill after N seconds
        assert_eq!(tools("timeout 30 curl https://example.com"), vec!["curl"]);
        // ionice: I/O scheduling class
        assert_eq!(tools("ionice -c 3 rsync -a /src/ /dst/"), vec!["rsync"]);
        // doas: OpenBSD sudo equivalent
        assert_eq!(tools("doas apt upgrade"), vec!["apt"]);
        // chrt: realtime scheduling
        assert_eq!(tools("chrt -f 99 dd if=/dev/zero of=/dev/null"), vec!["dd"]);
        // nohup survives logout
        assert_eq!(tools("nohup python3 train.py &"), vec!["python3"]);
        // chained wrappers: sudo + timeout + the real tool
        assert_eq!(tools("sudo timeout 60 rsync -avz /src /dst"), vec!["rsync"]);
    }

    #[test]
    fn path_stripping() {
        assert_eq!(tools("/usr/bin/grep -r foo ."), vec!["grep"]);
        assert_eq!(tools("./target/release/tapeworm log"), vec!["tapeworm"]);
    }

    #[test]
    fn empty_input() {
        assert_eq!(parse_pipeline("").len(), 0);
        assert_eq!(parse_pipeline("   ").len(), 0);
    }

    #[test]
    fn double_pipe_is_or() {
        let steps = parse_pipeline("cmd1 || cmd2");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].connector, "||");
    }

    #[test]
    fn extract_tool_empty() {
        assert_eq!(extract_tool("   "), "");
        assert_eq!(extract_tool("FOO=bar BAR=baz"), "");
    }
}

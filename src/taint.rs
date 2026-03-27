/// Forward taint analysis on shell pipelines.
///
/// Given a pipeline like:
///     curl --token <REDACTED> | jq .result | tee output.json
///
/// We classify each step with a `TaintLabel`:
///
/// | Label           | Meaning                                                         |
/// |-----------------|------------------------------------------------------------------|
/// | Clean           | No tainted data involved                                        |
/// | CredentialUse   | Has <REDACTED> args; tool sends secret externally (curl, ssh…) |
/// | TaintSource     | Has <REDACTED> args; stdout may carry secret data downstream    |
/// | Propagated      | Receives tainted stdin via |; passes it through                 |
/// | NetworkSink     | Receives tainted stdin via |; sends it to network              |
/// | FileSink        | Receives tainted stdin via |; writes it to disk                |
/// | ProcessSink     | Receives tainted stdin via |; spawns subprocesses with it      |
/// | Discarded       | Receives tainted stdin via |; output is metadata (taint stops) |
///
/// ## Propagation model
///
/// Only `|` carries taint (stdout → stdin).  `&&`, `||`, `;` are control-flow
/// operators that do not connect stdout to stdin, so taint does NOT propagate
/// across them.
///
/// When a Source step is a NetworkSink/FileSink/ProcessSink tool, the secret is
/// consumed as an *argument* (credential), not written to stdout, so the next
/// step's stdin is not tainted.  When the Source is a Passthrough/Unknown tool
/// (e.g. `echo <REDACTED>`), stdout IS the secret, so taint propagates.
///
/// ## Tool taxonomy
///
/// Conservative: unknown tools are treated as Passthrough (sound over-approximation).
/// We never miss a propagation path at the cost of possible false positives.

#[derive(Debug, Clone, Copy, PartialEq)]
enum TaintBehavior {
    /// stdout ≈ f(stdin): secret flows through — grep, head, jq, sed, awk, …
    Passthrough,
    /// stdout is metadata about stdin, not the data itself — wc, sha256sum, diff, …
    Discard,
    /// Sends data to an external endpoint; stdout is the response — curl, ssh, mysql, …
    NetworkSink,
    /// Writes data to disk; stdout is typically empty — dd, cp, rsync, …
    FileSink,
    /// Invokes subprocesses with stdin-derived arguments — xargs, parallel
    ProcessSink,
}

fn tool_behavior(tool: &str) -> TaintBehavior {
    match tool {
        // Passthrough — output is a transformation or subset of input
        "cat" | "grep" | "egrep" | "fgrep" | "rg" | "ripgrep" |
        "head" | "tail" | "sort" | "uniq" | "tee" |
        "less" | "more" | "most" |
        "cut" | "tr" | "sed" | "awk" | "gawk" | "mawk" |
        "jq" | "yq" | "xmlstarlet" | "xml" |
        "python" | "python3" | "perl" | "ruby" | "node" | "nodejs" |
        "sh" | "bash" | "zsh" | "dash" | "fish" |
        "strings" | "column" | "fmt" | "fold" | "expand" | "unexpand" |
        "rev" | "tac" | "paste" | "join" | "comm" |
        "base64" | "xxd" | "od" | "hexdump" |
        "gzip" | "gunzip" | "zcat" | "zgrep" |
        "bzip2" | "bunzip2" | "xz" | "unxz" | "lz4" | "zstd" |
        "pv" | "mbuffer" | "buffer" |
        "openssl" | "gpg" | "gpg2" |
        "iconv" | "uni2ascii" |
        "tput" => TaintBehavior::Passthrough,

        // Discard — output is about the data, not the data itself
        "wc" |
        "md5sum" | "sha1sum" | "sha224sum" | "sha256sum" |
        "sha384sum" | "sha512sum" | "b2sum" | "cksum" | "sum" |
        "diff" | "cmp" | "sdiff" | "wdiff" |
        "stat" | "file" | "du" => TaintBehavior::Discard,

        // Network sinks — consume stdin/args, send to external endpoint, stdout = response
        "curl" | "wget" | "http" | "httpie" | "fetch" | "aria2c" |
        "nc" | "netcat" | "ncat" | "socat" |
        "ssh" | "scp" | "sftp" | "ftp" | "ftps" |
        "telnet" | "rsh" | "rlogin" | "rcp" |
        "mysql" | "psql" | "sqlite3" | "redis-cli" | "mongo" | "mongosh" |
        "influx" | "clickhouse-client" |
        "mail" | "sendmail" | "mutt" | "msmtp" | "swaks" |
        "s3cmd" | "aws" | "gsutil" | "az" |
        "slack" | "notify-send" => TaintBehavior::NetworkSink,

        // File sinks — write to disk; stdout is empty or filename (tee handled in display)
        "dd" | "cp" | "mv" | "install" | "rsync" => TaintBehavior::FileSink,

        // Process sinks — spawn subprocesses with data as arguments
        "xargs" | "parallel" | "rush" => TaintBehavior::ProcessSink,

        // Conservative default: propagate (sound over-approximation)
        _ => TaintBehavior::Passthrough,
    }
}

/// Classification of a single pipeline step after taint propagation.
#[derive(Debug, Clone, PartialEq)]
pub enum TaintLabel {
    /// No tainted data involved.
    Clean,
    /// Step has `<REDACTED>` args; tool sends secret to network/disk/process.
    /// Stdout is NOT the secret, so taint does not propagate downstream.
    CredentialUse,
    /// Step has `<REDACTED>` args; stdout may carry secret data downstream.
    TaintSource,
    /// Receives tainted stdin via `|`; tool passes data through.
    Propagated,
    /// Receives tainted stdin via `|`; tool sends data to external network. ⚠
    NetworkSink,
    /// Receives tainted stdin via `|`; tool writes data to disk. ⚠
    FileSink,
    /// Receives tainted stdin via `|`; tool spawns subprocesses with tainted args. ⚠
    ProcessSink,
    /// Receives tainted stdin via `|`; tool discards data content (taint terminates).
    Discarded,
    /// Structural concern: a file-writing step follows a `CredentialUse` step in the
    /// same pipeline.  Taint did NOT propagate (the tool's stdout was not the secret),
    /// but the authenticated response is being written to disk.
    ///
    /// Example: `curl --token <REDACTED> https://api/endpoint | tee /tmp/out.json`
    ///   — tee receives HTTP response, not the token, but that response may be sensitive.
    ResponseSink,
}

impl TaintLabel {
    /// True if this label represents a security concern worth highlighting.
    pub fn is_warning(&self) -> bool {
        matches!(
            self,
            TaintLabel::CredentialUse
                | TaintLabel::NetworkSink
                | TaintLabel::FileSink
                | TaintLabel::ProcessSink
                | TaintLabel::ResponseSink
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TaintLabel::Clean         => "CLEAN",
            TaintLabel::CredentialUse => "CREDENTIAL-USE",
            TaintLabel::TaintSource   => "TAINT-SOURCE",
            TaintLabel::Propagated    => "PROPAGATED",
            TaintLabel::NetworkSink   => "NETWORK-SINK",
            TaintLabel::FileSink      => "FILE-SINK",
            TaintLabel::ProcessSink   => "PROCESS-SINK",
            TaintLabel::Discarded     => "DISCARDED",
            TaintLabel::ResponseSink  => "RESPONSE-SINK",
        }
    }
}

pub struct AnnotatedStep {
    pub step_index: i64,
    pub tool:       String,
    pub raw:        String,
    pub connector:  String,
    pub label:      TaintLabel,
}

pub struct TaintedPipeline {
    pub command_text:  String,
    pub timestamp_iso: String,
    pub steps:         Vec<AnnotatedStep>,
}

/// A raw row returned from the database query (one step per row, joined with command).
pub struct StepRow {
    pub command_id:    i64,
    pub command_text:  String,
    pub timestamp_iso: String,
    pub step_index:    i64,
    pub tool:          String,
    pub raw:           String,
    pub connector:     String,
}

/// Returns true if this tool writes its stdin/input to a file on disk.
///
/// Unlike `tool_behavior() == FileSink`, this also covers:
///   - `tee`  — FileSink by behavior is Passthrough, but it writes to disk
///   - `curl` / `wget` with `-o`/`-O`/`--output` flags
fn is_response_writer(tool: &str, raw: &str) -> bool {
    match tool_behavior(tool) {
        TaintBehavior::FileSink => true,
        _ => {
            tool == "tee"
                || (matches!(tool, "curl" | "wget")
                    && (raw.contains(" -o ")
                        || raw.contains(" -O ")
                        || raw.ends_with(" -O")
                        || raw.contains("--output")))
        }
    }
}

/// Annotate a sequence of pipeline steps (for one command, ordered by step_index).
///
/// Returns one `TaintLabel` per step.
fn annotate(steps: &[StepRow]) -> Vec<TaintLabel> {
    let mut labels = Vec::with_capacity(steps.len());
    // Does the current step receive tainted data on stdin?
    let mut stdin_tainted = false;

    for step in steps {
        let has_redacted = step.raw.contains("<REDACTED>");
        let behavior = tool_behavior(&step.tool);

        let label = if has_redacted {
            // This step is a taint source
            match behavior {
                // Tool sends secret to network/disk/process as a credential argument;
                // stdout is the response, not the secret — don't propagate.
                TaintBehavior::NetworkSink |
                TaintBehavior::FileSink    |
                TaintBehavior::ProcessSink => TaintLabel::CredentialUse,
                // Tool outputs data derived from secret (echo, grep, etc.) — propagate.
                _ => TaintLabel::TaintSource,
            }
        } else if stdin_tainted {
            match behavior {
                TaintBehavior::Passthrough => TaintLabel::Propagated,
                TaintBehavior::Discard     => TaintLabel::Discarded,
                TaintBehavior::NetworkSink => TaintLabel::NetworkSink,
                TaintBehavior::FileSink    => TaintLabel::FileSink,
                TaintBehavior::ProcessSink => TaintLabel::ProcessSink,
            }
        } else {
            TaintLabel::Clean
        };

        // Determine whether stdout of this step carries taint to the next step.
        // Only `|` pipes stdout → stdin.
        let stdout_tainted = matches!(label, TaintLabel::TaintSource | TaintLabel::Propagated);
        stdin_tainted = stdout_tainted && step.connector == "|";

        labels.push(label);
    }

    labels
}

/// Group raw DB rows by command_id, run `annotate()` on each group, and return
/// only pipelines that have at least one non-Clean step.
pub fn build_tainted_pipelines(rows: Vec<StepRow>) -> Vec<TaintedPipeline> {
    // Group by command_id (rows are already sorted by command_id, step_index)
    let mut pipelines: Vec<TaintedPipeline> = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let cid = rows[i].command_id;
        let j = rows[i..].iter().position(|r| r.command_id != cid)
            .map(|off| i + off)
            .unwrap_or(rows.len());

        let group = &rows[i..j];
        let labels = annotate(group);

        let mut steps: Vec<AnnotatedStep> = group.iter().zip(labels.iter())
            .map(|(row, label)| AnnotatedStep {
                step_index: row.step_index,
                tool:       row.tool.clone(),
                raw:        row.raw.clone(),
                connector:  row.connector.clone(),
                label:      label.clone(),
            })
            .collect();

        // Second pass: ResponseSink detection.
        //
        // A Clean step that writes to a file and follows a CredentialUse step in
        // the same pipeline may be storing an authenticated API response.  Taint
        // did not propagate (the credential was consumed as an argument, not
        // echoed to stdout), but the output is still potentially sensitive.
        //
        // Only `|` between the CredentialUse and the file-writer is required —
        // `&&`/`;` separation means the write is unrelated to the request.
        {
            let mut credential_use_pipe_active = false;
            for step in &mut steps {
                if step.label == TaintLabel::CredentialUse && step.connector == "|" {
                    credential_use_pipe_active = true;
                } else if credential_use_pipe_active {
                    if step.label == TaintLabel::Clean
                        && is_response_writer(&step.tool, &step.raw)
                    {
                        step.label = TaintLabel::ResponseSink;
                    }
                    // Chain continues as long as we keep seeing | connectors
                    if step.connector != "|" {
                        credential_use_pipe_active = false;
                    }
                } else {
                    credential_use_pipe_active = false;
                }
            }
        }

        // Only include pipelines that have at least one non-Clean step
        if steps.iter().any(|s| s.label != TaintLabel::Clean) {
            pipelines.push(TaintedPipeline {
                command_text:  group[0].command_text.clone(),
                timestamp_iso: group[0].timestamp_iso.clone(),
                steps,
            });
        }

        i = j;
    }
    pipelines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_steps(specs: &[(&str, &str, &str)]) -> Vec<StepRow> {
        specs.iter().enumerate().map(|(i, (tool, raw, connector))| StepRow {
            command_id:    1,
            command_text:  String::new(),
            timestamp_iso: String::new(),
            step_index:    i as i64,
            tool:          tool.to_string(),
            raw:           raw.to_string(),
            connector:     connector.to_string(),
        }).collect()
    }

    fn labels(specs: &[(&str, &str, &str)]) -> Vec<TaintLabel> {
        annotate(&make_steps(specs))
    }

    // ── Credential use: secret as argument to sink tool ─────────────────────

    #[test]
    fn curl_credential_does_not_propagate() {
        // curl --token <REDACTED> | jq .
        // jq receives the HTTP response, not the token
        let ls = labels(&[
            ("curl", "curl --token <REDACTED> https://api.example.com", "|"),
            ("jq",   "jq .",                                             ""),
        ]);
        assert_eq!(ls[0], TaintLabel::CredentialUse);
        assert_eq!(ls[1], TaintLabel::Clean);
    }

    #[test]
    fn mysql_credential_does_not_propagate() {
        let ls = labels(&[
            ("mysql", "mysql -p <REDACTED> -e 'SELECT 1'", "|"),
            ("grep",  "grep 1",                             ""),
        ]);
        assert_eq!(ls[0], TaintLabel::CredentialUse);
        assert_eq!(ls[1], TaintLabel::Clean);
    }

    // ── Taint source: secret in passthrough tool stdout ──────────────────────

    #[test]
    fn echo_secret_propagates() {
        // echo <REDACTED> | base64 | curl -d @- http://host
        let ls = labels(&[
            ("echo",   "echo <REDACTED>",         "|"),
            ("base64", "base64",                   "|"),
            ("curl",   "curl -d @- http://host",   ""),
        ]);
        assert_eq!(ls[0], TaintLabel::TaintSource);
        assert_eq!(ls[1], TaintLabel::Propagated);
        assert_eq!(ls[2], TaintLabel::NetworkSink);
    }

    #[test]
    fn grep_secret_pattern_propagates() {
        // grep <REDACTED> logfile | head
        let ls = labels(&[
            ("grep", "grep <REDACTED> logfile", "|"),
            ("head", "head",                     ""),
        ]);
        assert_eq!(ls[0], TaintLabel::TaintSource);
        assert_eq!(ls[1], TaintLabel::Propagated);
    }

    // ── Propagation stops at discard ────────────────────────────────────────

    #[test]
    fn wc_discards_taint() {
        let ls = labels(&[
            ("echo", "echo <REDACTED>", "|"),
            ("wc",   "wc -c",           "|"),
            ("grep", "grep .",           ""),
        ]);
        assert_eq!(ls[0], TaintLabel::TaintSource);
        assert_eq!(ls[1], TaintLabel::Discarded);
        assert_eq!(ls[2], TaintLabel::Clean);
    }

    // ── Non-pipe connectors don't carry taint ────────────────────────────────

    #[test]
    fn and_connector_does_not_propagate() {
        // echo <REDACTED> && cat /etc/passwd
        let ls = labels(&[
            ("echo", "echo <REDACTED>",  "&&"),
            ("cat",  "cat /etc/passwd",   ""),
        ]);
        assert_eq!(ls[0], TaintLabel::TaintSource);
        assert_eq!(ls[1], TaintLabel::Clean); // && doesn't pipe stdin
    }

    #[test]
    fn semicolon_does_not_propagate() {
        let ls = labels(&[
            ("echo", "echo <REDACTED>", ";"),
            ("ls",   "ls",               ""),
        ]);
        assert_eq!(ls[0], TaintLabel::TaintSource);
        assert_eq!(ls[1], TaintLabel::Clean);
    }

    // ── Multi-hop propagation ────────────────────────────────────────────────

    #[test]
    fn multi_hop_propagation() {
        // echo <REDACTED> | sed s/x/y/ | awk '{print}' | curl -d @- http://host
        let ls = labels(&[
            ("echo", "echo <REDACTED>",      "|"),
            ("sed",  "sed s/x/y/",            "|"),
            ("awk",  "awk '{print}'",          "|"),
            ("curl", "curl -d @- http://host", ""),
        ]);
        assert_eq!(ls[0], TaintLabel::TaintSource);
        assert_eq!(ls[1], TaintLabel::Propagated);
        assert_eq!(ls[2], TaintLabel::Propagated);
        assert_eq!(ls[3], TaintLabel::NetworkSink);
    }

    // ── File sink ────────────────────────────────────────────────────────────

    #[test]
    fn tee_propagates_and_file_write_flagged_in_display() {
        // tee is Passthrough — the display layer notes the file write separately
        let ls = labels(&[
            ("echo", "echo <REDACTED>",  "|"),
            ("tee",  "tee /tmp/out.txt",  "|"),
            ("grep", "grep .",             ""),
        ]);
        assert_eq!(ls[0], TaintLabel::TaintSource);
        assert_eq!(ls[1], TaintLabel::Propagated); // tee passes through; display adds note
        assert_eq!(ls[2], TaintLabel::Propagated);
    }

    // ── ResponseSink: authenticated response written to disk ────────────────
    //
    // These tests must go through `build_tainted_pipelines` because the second
    // pass runs there, not in `annotate`.

    /// Build a single-pipeline from specs and return its step labels.
    fn full_labels(specs: &[(&str, &str, &str)]) -> Vec<TaintLabel> {
        let rows = make_steps(specs);
        let pipelines = build_tainted_pipelines(rows);
        // If no pipeline was produced (all clean after both passes), return all Clean
        if pipelines.is_empty() {
            return vec![TaintLabel::Clean; specs.len()];
        }
        pipelines.into_iter().next().unwrap().steps
            .into_iter().map(|s| s.label).collect()
    }

    #[test]
    fn tee_after_credential_use_is_response_sink() {
        // curl --token <REDACTED> | jq | tee /tmp/out.json
        // jq and tee receive the HTTP response, not the token.
        // tee writes the response to disk → ResponseSink.
        let ls = full_labels(&[
            ("curl", "curl --token <REDACTED> https://api.example.com", "|"),
            ("jq",   "jq .result",                                       "|"),
            ("tee",  "tee /tmp/out.json",                                 ""),
        ]);
        assert_eq!(ls[0], TaintLabel::CredentialUse);
        assert_eq!(ls[1], TaintLabel::Clean);
        assert_eq!(ls[2], TaintLabel::ResponseSink);
    }

    #[test]
    fn direct_tee_after_credential_use() {
        // curl --token <REDACTED> | tee /tmp/out.json  (no jq in between)
        let ls = full_labels(&[
            ("curl", "curl --token <REDACTED> https://api.example.com", "|"),
            ("tee",  "tee /tmp/out.json",                                 ""),
        ]);
        assert_eq!(ls[0], TaintLabel::CredentialUse);
        assert_eq!(ls[1], TaintLabel::ResponseSink);
    }

    #[test]
    fn and_separated_file_write_is_not_response_sink() {
        // curl --token <REDACTED> https://api && cp /tmp/existing /tmp/copy
        // The cp is not receiving the curl response — they're separate commands.
        let ls = full_labels(&[
            ("curl", "curl --token <REDACTED> https://api", "&&"),
            ("cp",   "cp /tmp/existing /tmp/copy",           ""),
        ]);
        assert_eq!(ls[0], TaintLabel::CredentialUse);
        assert_eq!(ls[1], TaintLabel::Clean); // && breaks the pipeline
    }

    #[test]
    fn wget_output_flag_is_response_sink() {
        // wget --password <REDACTED> -O /tmp/data.xml
        // Single step: credential use + output-to-file.
        // wget is CredentialUse (first pass); the ResponseSink second pass applies
        // only to downstream Clean steps — wget itself stays CredentialUse.
        // (The -O behavior is noted in the tee note in display for now.)
        let ls = full_labels(&[
            ("wget", "wget --password <REDACTED> -O /tmp/data.xml", ""),
        ]);
        assert_eq!(ls[0], TaintLabel::CredentialUse);
    }

    #[test]
    fn dd_after_credential_use_is_response_sink() {
        // curl --token <REDACTED> | dd of=/tmp/dump.bin
        let ls = full_labels(&[
            ("curl", "curl --token <REDACTED> https://api/binary", "|"),
            ("dd",   "dd of=/tmp/dump.bin",                         ""),
        ]);
        assert_eq!(ls[0], TaintLabel::CredentialUse);
        assert_eq!(ls[1], TaintLabel::ResponseSink);
    }

    // ── No redacted steps → no taint ────────────────────────────────────────

    #[test]
    fn clean_pipeline() {
        let ls = labels(&[
            ("grep", "grep foo bar.txt", "|"),
            ("sort", "sort",              "|"),
            ("uniq", "uniq -c",           ""),
        ]);
        assert!(ls.iter().all(|l| *l == TaintLabel::Clean));
    }
}

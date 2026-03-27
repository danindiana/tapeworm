use crate::archetype::{Archetype, BaselineStats, Classification, SessionFeatures};
use crate::db::{SessionSummary, ToolEdge};
use crate::record::CommandRecord;
use crate::taint::{TaintLabel, TaintedPipeline};
use chrono::{Local, TimeZone};
use colored::Colorize;
use comfy_table::{Attribute, Cell, Color, Table};

pub fn print_log(records: &[CommandRecord]) {
    if records.is_empty() {
        println!("{}", "No records found.".yellow());
        return;
    }

    let mut table = Table::new();
    table.set_header(vec![
        Cell::new("Time").add_attribute(Attribute::Bold),
        Cell::new("Exit").add_attribute(Attribute::Bold),
        Cell::new("ms").add_attribute(Attribute::Bold),
        Cell::new("Dir").add_attribute(Attribute::Bold),
        Cell::new("Command").add_attribute(Attribute::Bold),
    ]);

    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    for r in records {
        let exit_cell = if r.exit_code == 0 {
            Cell::new("0").fg(Color::Green)
        } else {
            Cell::new(r.exit_code.to_string()).fg(Color::Red)
        };

        let cmd_display = if r.command.len() > 80 {
            format!("{}…", &r.command[..79])
        } else {
            r.command.clone()
        };

        let cwd_display = if !home.is_empty() && r.cwd.starts_with(&home) {
            format!("~{}", &r.cwd[home.len()..])
        } else {
            r.cwd.clone()
        };

        // Trim ISO timestamp to "2026-03-26T14:05:00" — sub-second precision wastes width
        let ts = if r.timestamp_iso.len() >= 19 {
            &r.timestamp_iso[..19]
        } else {
            &r.timestamp_iso
        };

        table.add_row(vec![
            Cell::new(ts),
            exit_cell,
            Cell::new(r.duration_ms.to_string()),
            Cell::new(cwd_display),
            Cell::new(cmd_display),
        ]);
    }

    println!("{table}");
}

pub fn print_stats(
    total: i64,
    avg_ms: f64,
    top_cmds: &[(String, i64)],
    hourly: &[(i64, i64)],
) {
    println!("{}", "=== tapeworm stats ===".bold().cyan());
    println!("Total commands recorded : {}", total.to_string().yellow());
    println!("Average duration        : {:.1} ms", avg_ms);
    println!();

    println!("{}", "Top commands:".bold());
    let mut tbl = Table::new();
    tbl.set_header(vec![
        Cell::new("#").add_attribute(Attribute::Bold),
        Cell::new("Count").add_attribute(Attribute::Bold),
        Cell::new("Command").add_attribute(Attribute::Bold),
    ]);
    for (i, (cmd, cnt)) in top_cmds.iter().enumerate() {
        let cmd_display = if cmd.len() > 60 {
            format!("{}…", &cmd[..59])
        } else {
            cmd.clone()
        };
        tbl.add_row(vec![
            Cell::new((i + 1).to_string()),
            Cell::new(cnt.to_string()).fg(Color::Yellow),
            Cell::new(cmd_display),
        ]);
    }
    println!("{tbl}");
    println!();

    println!("{}", "Activity by hour (local):".bold());

    let max_count = hourly.iter().map(|(_, c)| *c).max().unwrap_or(1);
    for h in 0i64..24 {
        let count = hourly
            .iter()
            .find(|(hr, _)| *hr == h)
            .map(|(_, c)| *c)
            .unwrap_or(0);
        let bar_len = if max_count > 0 {
            ((count as f64 / max_count as f64) * 40.0) as usize
        } else {
            0
        };
        let bar = "#".repeat(bar_len);
        println!("{:02}:00  {:>5}  {}", h, count, bar.green());
    }
}

pub fn print_sessions(sessions: &[SessionSummary]) {
    if sessions.is_empty() {
        println!("{}", "No sessions recorded yet.".yellow());
        return;
    }
    println!("{}", "=== sessions ===".bold().cyan());
    let mut tbl = Table::new();
    tbl.set_header(vec![
        Cell::new("Session").add_attribute(Attribute::Bold),
        Cell::new("Started").add_attribute(Attribute::Bold),
        Cell::new("Duration").add_attribute(Attribute::Bold),
        Cell::new("Cmds").add_attribute(Attribute::Bold),
        Cell::new("Fails").add_attribute(Attribute::Bold),
        Cell::new("Shell").add_attribute(Attribute::Bold),
    ]);
    for s in sessions {
        let started = Local.timestamp_opt(s.start_unix, 0)
            .single()
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| s.start_unix.to_string());
        let wall_secs = (s.end_unix - s.start_unix).max(0);
        let duration = fmt_duration(wall_secs);
        let fail_cell = if s.failure_count > 0 {
            Cell::new(s.failure_count.to_string()).fg(Color::Red)
        } else {
            Cell::new("0").fg(Color::Green)
        };
        tbl.add_row(vec![
            Cell::new(&s.session_id[..8.min(s.session_id.len())]),
            Cell::new(started),
            Cell::new(duration),
            Cell::new(s.cmd_count.to_string()).fg(Color::Yellow),
            fail_cell,
            Cell::new(&s.shell),
        ]);
    }
    println!("{tbl}");
}

pub fn print_session_timeline(session_id: &str, records: &[CommandRecord]) {
    if records.is_empty() {
        println!("{}", "No commands found for that session.".yellow());
        return;
    }
    println!("{} {}", "=== session".bold().cyan(), session_id.cyan().bold());
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut tbl = Table::new();
    tbl.set_header(vec![
        Cell::new("#").add_attribute(Attribute::Bold),
        Cell::new("Time").add_attribute(Attribute::Bold),
        Cell::new("Exit").add_attribute(Attribute::Bold),
        Cell::new("dur").add_attribute(Attribute::Bold),
        Cell::new("gap").add_attribute(Attribute::Bold),
        Cell::new("Dir").add_attribute(Attribute::Bold),
        Cell::new("Command").add_attribute(Attribute::Bold),
    ]);
    for (i, r) in records.iter().enumerate() {
        let exit_cell = if r.exit_code == 0 {
            Cell::new("0").fg(Color::Green)
        } else {
            Cell::new(r.exit_code.to_string()).fg(Color::Red)
        };
        let cmd_display = if r.command.len() > 70 {
            format!("{}…", &r.command[..69])
        } else {
            r.command.clone()
        };
        let cwd_display = if !home.is_empty() && r.cwd.starts_with(&home) {
            format!("~{}", &r.cwd[home.len()..])
        } else {
            r.cwd.clone()
        };
        let ts = if r.timestamp_iso.len() >= 19 { &r.timestamp_iso[11..19] } else { &r.timestamp_iso };
        // Colour gap by magnitude: >60s = yellow (long think), >300s = red (long idle)
        let gap_cell = if r.gap_ms == 0 {
            Cell::new("-").fg(Color::DarkGrey)
        } else if r.gap_ms >= 300_000 {
            Cell::new(fmt_gap(r.gap_ms)).fg(Color::Red)
        } else if r.gap_ms >= 60_000 {
            Cell::new(fmt_gap(r.gap_ms)).fg(Color::Yellow)
        } else {
            Cell::new(fmt_gap(r.gap_ms)).fg(Color::DarkGrey)
        };
        tbl.add_row(vec![
            Cell::new((i + 1).to_string()),
            Cell::new(ts),
            exit_cell,
            Cell::new(r.duration_ms.to_string()),
            gap_cell,
            Cell::new(cwd_display),
            Cell::new(cmd_display),
        ]);
    }
    println!("{tbl}");
    print_gap_histogram(records);
}

pub fn print_failure_chains(pairs: &[(CommandRecord, CommandRecord)]) {
    if pairs.is_empty() {
        println!("{}", "No failure chains found.".yellow());
        return;
    }
    println!("{}", "=== failure chains (failed → next command) ===".bold().cyan());
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let shorten_cwd = |cwd: &str| -> String {
        if !home.is_empty() && cwd.starts_with(&home) {
            format!("~{}", &cwd[home.len()..])
        } else {
            cwd.to_string()
        }
    };
    let mut tbl = Table::new();
    tbl.set_header(vec![
        Cell::new("Failed command").add_attribute(Attribute::Bold),
        Cell::new("Exit").add_attribute(Attribute::Bold),
        Cell::new("Dir").add_attribute(Attribute::Bold),
        Cell::new("→ Next command").add_attribute(Attribute::Bold),
    ]);
    for (failed, next) in pairs {
        let f_cmd = if failed.command.len() > 50 {
            format!("{}…", &failed.command[..49])
        } else {
            failed.command.clone()
        };
        let n_cmd = if next.command.len() > 50 {
            format!("{}…", &next.command[..49])
        } else {
            next.command.clone()
        };
        tbl.add_row(vec![
            Cell::new(f_cmd).fg(Color::Red),
            Cell::new(failed.exit_code.to_string()).fg(Color::Red),
            Cell::new(shorten_cwd(&failed.cwd)),
            Cell::new(n_cmd).fg(Color::Cyan),
        ]);
    }
    println!("{tbl}");
}

/// Horizontal bar chart of gap_ms bucket distribution for a session.
/// Only rendered when at least one gap_ms > 0.
fn print_gap_histogram(records: &[CommandRecord]) {
    let gaps: Vec<i64> = records.iter().filter(|r| r.gap_ms > 0).map(|r| r.gap_ms).collect();
    if gaps.is_empty() {
        return;
    }

    const BUCKETS: &[(&str, i64, i64)] = &[
        ("<1s",   0,           1_000),
        ("1-5s",  1_000,       5_000),
        ("5-30s", 5_000,       30_000),
        ("30-60s",30_000,      60_000),
        ("1-5m",  60_000,      300_000),
        (">5m",   300_000,     i64::MAX),
    ];

    let counts: Vec<usize> = BUCKETS.iter()
        .map(|(_, lo, hi)| gaps.iter().filter(|&&g| g >= *lo && g < *hi).count())
        .collect();
    let max = counts.iter().copied().max().unwrap_or(1).max(1);

    println!("{}", "  gap distribution:".dimmed());
    for (i, (label, _, _)) in BUCKETS.iter().enumerate() {
        let n = counts[i];
        let bar_len = if n > 0 { (n * 24 / max).max(1) } else { 0 };
        let bar = "▪".repeat(bar_len);
        println!(
            "  {:>6}  {:>3}  {}",
            label.dimmed(),
            n.to_string().dimmed(),
            if n > 0 { bar.yellow().to_string() } else { String::new() }
        );
    }
    println!();
}

/// Format a gap in milliseconds as a human-readable string: "4.2s", "2m3s".
fn fmt_gap(ms: i64) -> String {
    if ms < 1_000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else if ms < 3_600_000 {
        format!("{}m{}s", ms / 60_000, (ms % 60_000) / 1000)
    } else {
        format!("{}h{}m", ms / 3_600_000, (ms % 3_600_000) / 60_000)
    }
}

fn fmt_duration(secs: i64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub fn print_semantic_results(records: &[CommandRecord], scores: &[(i64, f32)]) {
    if records.is_empty() {
        println!("{}", "No matching commands found.".yellow());
        return;
    }

    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut table = Table::new();
    table.set_header(vec![
        Cell::new("Sim").add_attribute(Attribute::Bold),
        Cell::new("Time").add_attribute(Attribute::Bold),
        Cell::new("Exit").add_attribute(Attribute::Bold),
        Cell::new("Dir").add_attribute(Attribute::Bold),
        Cell::new("Command").add_attribute(Attribute::Bold),
    ]);

    for r in records {
        let score = r.id
            .and_then(|id| scores.iter().find(|(sid, _)| *sid == id))
            .map(|(_, s)| *s)
            .unwrap_or(0.0);

        let sim_pct = (score * 100.0) as u32;
        let sim_cell = if sim_pct >= 80 {
            Cell::new(format!("{sim_pct}%")).fg(Color::Green)
        } else if sim_pct >= 60 {
            Cell::new(format!("{sim_pct}%")).fg(Color::Yellow)
        } else {
            Cell::new(format!("{sim_pct}%")).fg(Color::DarkGrey)
        };

        let exit_cell = if r.exit_code == 0 {
            Cell::new("0").fg(Color::Green)
        } else {
            Cell::new(r.exit_code.to_string()).fg(Color::Red)
        };

        let cmd_display = if r.command.len() > 80 {
            format!("{}…", &r.command[..79])
        } else {
            r.command.clone()
        };

        let cwd_display = if !home.is_empty() && r.cwd.starts_with(&home) {
            format!("~{}", &r.cwd[home.len()..])
        } else {
            r.cwd.clone()
        };

        let ts = if r.timestamp_iso.len() >= 19 { &r.timestamp_iso[..19] } else { &r.timestamp_iso };

        table.add_row(vec![
            sim_cell,
            Cell::new(ts),
            exit_cell,
            Cell::new(cwd_display),
            Cell::new(cmd_display),
        ]);
    }

    println!("{table}");
}

pub fn print_tools(top_tools: &[(String, i64)]) {
    if top_tools.is_empty() {
        println!("{}", "No pipeline step data yet. Run some commands first.".yellow());
        return;
    }
    println!("{}", "=== top tools (by pipeline step frequency) ===".bold().cyan());
    let mut tbl = Table::new();
    tbl.set_header(vec![
        Cell::new("#").add_attribute(Attribute::Bold),
        Cell::new("Count").add_attribute(Attribute::Bold),
        Cell::new("Tool").add_attribute(Attribute::Bold),
        Cell::new("").add_attribute(Attribute::Bold), // bar column
    ]);
    let max = top_tools.first().map(|(_, c)| *c).unwrap_or(1);
    for (i, (tool, cnt)) in top_tools.iter().enumerate() {
        let bar_len = ((*cnt as f64 / max as f64) * 30.0) as usize;
        tbl.add_row(vec![
            Cell::new((i + 1).to_string()),
            Cell::new(cnt.to_string()).fg(Color::Yellow),
            Cell::new(tool),
            Cell::new("#".repeat(bar_len)).fg(Color::Green),
        ]);
    }
    println!("{tbl}");
}

pub fn print_pipes(patterns: &[(String, i64)], bigrams: &[(String, String, i64)]) {
    println!("{}", "=== top pipeline patterns ===".bold().cyan());
    if patterns.is_empty() {
        println!("{}", "No multi-step pipelines recorded yet.".yellow());
    } else {
        let mut tbl = Table::new();
        tbl.set_header(vec![
            Cell::new("#").add_attribute(Attribute::Bold),
            Cell::new("Count").add_attribute(Attribute::Bold),
            Cell::new("Pipeline").add_attribute(Attribute::Bold),
        ]);
        for (i, (pat, cnt)) in patterns.iter().enumerate() {
            let display = if pat.len() > 70 {
                format!("{}…", &pat[..69])
            } else {
                pat.clone()
            };
            tbl.add_row(vec![
                Cell::new((i + 1).to_string()),
                Cell::new(cnt.to_string()).fg(Color::Yellow),
                Cell::new(display).fg(Color::Cyan),
            ]);
        }
        println!("{tbl}");
    }

    println!();
    println!("{}", "=== top pipe bigrams  (A | B) ===".bold().cyan());
    if bigrams.is_empty() {
        println!("{}", "No pipe bigrams recorded yet.".yellow());
    } else {
        let mut tbl = Table::new();
        tbl.set_header(vec![
            Cell::new("#").add_attribute(Attribute::Bold),
            Cell::new("Count").add_attribute(Attribute::Bold),
            Cell::new("From").add_attribute(Attribute::Bold),
            Cell::new("→").add_attribute(Attribute::Bold),
            Cell::new("To").add_attribute(Attribute::Bold),
        ]);
        for (i, (from, to, cnt)) in bigrams.iter().enumerate() {
            tbl.add_row(vec![
                Cell::new((i + 1).to_string()),
                Cell::new(cnt.to_string()).fg(Color::Yellow),
                Cell::new(from).fg(Color::Green),
                Cell::new("→"),
                Cell::new(to).fg(Color::Green),
            ]);
        }
        println!("{tbl}");
    }
}

/// Session archetype classification table.
pub fn print_archetypes(pairs: &[(SessionFeatures, Classification)], baseline: Option<&BaselineStats>) {
    if pairs.is_empty() {
        println!("{}", "=== session archetypes ===".bold().cyan());
        println!("{}", "No sessions recorded yet.".yellow());
        return;
    }

    println!("{}", "=== session archetypes ===".bold().cyan());
    let mut tbl = Table::new();
    tbl.set_header(vec![
        Cell::new("Session").add_attribute(Attribute::Bold),
        Cell::new("Started").add_attribute(Attribute::Bold),
        Cell::new("Shell").add_attribute(Attribute::Bold),
        Cell::new("Cmds").add_attribute(Attribute::Bold),
        Cell::new("Archetype").add_attribute(Attribute::Bold),
        Cell::new("fail%").add_attribute(Attribute::Bold),
        Cell::new("gap̄").add_attribute(Attribute::Bold),
        Cell::new("entropy").add_attribute(Attribute::Bold),
        Cell::new("flags").add_attribute(Attribute::Bold),
    ]);

    for (f, c) in pairs {
        let started = Local.timestamp_opt(f.start_unix, 0)
            .single()
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| f.start_unix.to_string());

        let archetype_cell = match &c.archetype {
            Archetype::Unknown     => Cell::new("unknown").fg(Color::DarkGrey),
            Archetype::Burst       => Cell::new("burst").fg(Color::Cyan),
            Archetype::Debugging   => Cell::new("debugging").fg(Color::Red),
            Archetype::Focused     => Cell::new("focused").fg(Color::Green),
            Archetype::Exploratory => Cell::new("exploratory").fg(Color::Yellow),
        };

        let fail_pct = format!("{:.0}%", f.failure_rate * 100.0);
        let fail_cell = if f.failure_rate > 0.35 {
            Cell::new(fail_pct).fg(Color::Red)
        } else if f.failure_rate > 0.1 {
            Cell::new(fail_pct).fg(Color::Yellow)
        } else {
            Cell::new(fail_pct).fg(Color::Green)
        };

        let gap_display = if f.mean_gap_ms > 0.0 {
            fmt_gap(f.mean_gap_ms as i64)
        } else {
            "-".to_string()
        };

        let entropy_display = if f.tool_entropy > 0.0 {
            format!("{:.2}", f.tool_entropy)
        } else {
            "-".to_string()
        };

        // Assemble flags: interrupted warning + σ deviation indicators
        let mut flag_parts: Vec<String> = Vec::new();
        if c.interrupted {
            flag_parts.push("⚠ interrupted".to_string());
        }
        if let Some(b) = baseline {
            if b.failure_outlier(f.failure_rate) {
                flag_parts.push(if f.failure_rate > b.failure_mean {
                    "↑fail".to_string()
                } else {
                    "↓fail".to_string()
                });
            }
            if b.gap_outlier(f.mean_gap_ms) {
                flag_parts.push(if f.mean_gap_ms > b.gap_mean {
                    "↑gap".to_string()
                } else {
                    "↓gap".to_string()
                });
            }
            if b.entropy_outlier(f.tool_entropy) {
                flag_parts.push(if f.tool_entropy > b.entropy_mean {
                    "↑ent".to_string()
                } else {
                    "↓ent".to_string()
                });
            }
        }
        let flags = flag_parts.join(" ");

        let sid = &f.session_id;
        let sid_short = &sid[..8.min(sid.len())];

        tbl.add_row(vec![
            Cell::new(sid_short),
            Cell::new(started),
            Cell::new(&f.shell).fg(Color::DarkGrey),
            Cell::new(f.cmd_count.to_string()).fg(Color::Yellow),
            archetype_cell,
            fail_cell,
            Cell::new(gap_display).fg(Color::DarkGrey),
            Cell::new(entropy_display).fg(Color::DarkGrey),
            Cell::new(&flags).fg(Color::Red),
        ]);
    }
    println!("{tbl}");
    let mut legend = "  burst=fast gaps  debugging=high fail  focused=low entropy  exploratory=high entropy".to_string();
    if baseline.is_some() {
        legend.push_str("  ↑↓=2σ outlier");
    }
    println!("{}", legend.dimmed());
}

/// Detailed classification explanation for a single session.
/// Shows each decision gate with its feature value, threshold, and outcome.
pub fn print_archetype_explain(f: &SessionFeatures, c: &Classification) {
    let sid_short = &f.session_id[..8.min(f.session_id.len())];
    println!(
        "{} {}",
        "=== archetype explain:".bold().cyan(),
        sid_short.cyan().bold()
    );

    let started = Local.timestamp_opt(f.start_unix, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| f.start_unix.to_string());

    println!(
        "  session {}   started {}   shell {}   commands {}",
        sid_short.dimmed(),
        started.dimmed(),
        f.shell.dimmed(),
        f.cmd_count.to_string().yellow()
    );
    if c.interrupted {
        println!("  {}", "⚠  session was interrupted (gap ≥ 5 min)".truecolor(255, 165, 0));
    }
    println!();

    // Feature summary
    println!("{}", "Features:".bold());
    println!(
        "  {:<16} {}",
        "cmd_count".dimmed(),
        f.cmd_count.to_string().yellow()
    );
    println!(
        "  {:<16} {}",
        "failure_rate".dimmed(),
        format!("{:.1}%", f.failure_rate * 100.0).yellow()
    );
    let gap_str = if f.mean_gap_ms > 0.0 { fmt_gap(f.mean_gap_ms as i64) } else { "-".to_string() };
    println!(
        "  {:<16} {}  (max: {})",
        "mean_gap".dimmed(),
        gap_str.yellow(),
        fmt_gap(f.max_gap_ms).dimmed()
    );
    let ent_str = if f.tool_entropy > 0.0 { format!("{:.3}", f.tool_entropy) } else { "-".to_string() };
    println!("  {:<16} {}", "tool_entropy".dimmed(), ent_str.yellow());
    println!();

    // Decision path — mirror the classify() rule order
    println!("{}", "Decision path:".bold());

    // Gate 1: unknown check
    let gate1_pass = f.cmd_count >= 3;
    println!(
        "  {} cmd_count ≥ 3?           {}  →  {}",
        if gate1_pass { "✓".green() } else { "✗".red() },
        format!("{} ≥ 3", f.cmd_count).dimmed(),
        if gate1_pass { "continue".dimmed() } else { "UNKNOWN  ◀".red().bold() }
    );
    if !gate1_pass {
        println!();
        println!("  {}", format!("Classification: UNKNOWN  (too few commands)").bold());
        return;
    }

    // Gate 2: debugging
    let gate2_fire = f.failure_rate > 0.35;
    println!(
        "  {} failure_rate > 35%?       {}  →  {}",
        if gate2_fire { "✓".green() } else { "✗".dimmed() },
        format!("{:.1}% > 35%", f.failure_rate * 100.0).dimmed(),
        if gate2_fire { "DEBUGGING  ◀".red().bold() } else { "skip".dimmed() }
    );
    if gate2_fire {
        println!();
        println!("  {}", "Classification: DEBUGGING".bold().red());
        return;
    }

    // Gate 3: burst
    let gap_ok = f.mean_gap_ms > 0.0 && f.mean_gap_ms < 2_000.0;
    let cnt_ok  = f.cmd_count >= 5;
    let gate3_fire = gap_ok && cnt_ok;
    println!(
        "  {} gap < 2s AND cmds ≥ 5?   {}  AND  {}  →  {}",
        if gate3_fire { "✓".green() } else { "✗".dimmed() },
        if f.mean_gap_ms > 0.0 {
            format!("{} < 2s {}", fmt_gap(f.mean_gap_ms as i64), if gap_ok { "✓" } else { "✗" }).dimmed()
        } else {
            "no gap data ✗".dimmed()
        },
        format!("{} ≥ 5 {}", f.cmd_count, if cnt_ok { "✓" } else { "✗" }).dimmed(),
        if gate3_fire { "BURST  ◀".cyan().bold() } else { "skip".dimmed() }
    );
    if gate3_fire {
        println!();
        println!("  {}", "Classification: BURST".bold().cyan());
        return;
    }

    // Gate 4: focused
    let gate4_fire = f.tool_entropy > 0.0 && f.tool_entropy < 0.45;
    println!(
        "  {} entropy > 0 AND < 0.45?  {}  →  {}",
        if gate4_fire { "✓".green() } else { "✗".dimmed() },
        format!("{:.3} < 0.45", f.tool_entropy).dimmed(),
        if gate4_fire { "FOCUSED  ◀".green().bold() } else { "skip".dimmed() }
    );
    if gate4_fire {
        println!();
        println!("  {}", "Classification: FOCUSED".bold().green());
        return;
    }

    // Gate 5: exploratory
    let gate5_fire = f.tool_entropy >= 0.45;
    println!(
        "  {} entropy ≥ 0.45?           {}  →  {}",
        if gate5_fire { "✓".green() } else { "✗".dimmed() },
        format!("{:.3} ≥ 0.45", f.tool_entropy).dimmed(),
        if gate5_fire { "EXPLORATORY  ◀".yellow().bold() } else { "skip".dimmed() }
    );

    println!();
    let archetype_str = match &c.archetype {
        Archetype::Unknown     => "UNKNOWN (no gap data, no pipeline steps)".dimmed().to_string(),
        Archetype::Burst       => "BURST".cyan().bold().to_string(),
        Archetype::Debugging   => "DEBUGGING".red().bold().to_string(),
        Archetype::Focused     => "FOCUSED".green().bold().to_string(),
        Archetype::Exploratory => "EXPLORATORY".yellow().bold().to_string(),
    };
    println!("  Classification: {}", archetype_str);
}

/// Compact one-line archetype summary for `session show` footer.
pub fn print_session_archetype_summary(f: &SessionFeatures, c: &Classification) {
    let archetype_str = match &c.archetype {
        Archetype::Unknown     => "unknown".dimmed().to_string(),
        Archetype::Burst       => "burst".cyan().bold().to_string(),
        Archetype::Debugging   => "debugging".red().bold().to_string(),
        Archetype::Focused     => "focused".green().bold().to_string(),
        Archetype::Exploratory => "exploratory".yellow().bold().to_string(),
    };
    let fail_pct = format!("{:.0}%", f.failure_rate * 100.0);
    let fail_str = if f.failure_rate > 0.35 {
        fail_pct.red().to_string()
    } else if f.failure_rate > 0.1 {
        fail_pct.yellow().to_string()
    } else {
        fail_pct.green().to_string()
    };
    let gap_str = if f.mean_gap_ms > 0.0 {
        fmt_gap(f.mean_gap_ms as i64).dimmed().to_string()
    } else {
        "-".dimmed().to_string()
    };
    let ent_str = if f.tool_entropy > 0.0 {
        format!("{:.2}", f.tool_entropy).dimmed().to_string()
    } else {
        "-".dimmed().to_string()
    };
    let interrupted = if c.interrupted { "  ⚠ interrupted".truecolor(255, 165, 0).to_string() } else { String::new() };
    println!(
        "  archetype {}   fail {}   gap̄ {}   entropy {}{}",
        archetype_str, fail_str, gap_str, ent_str, interrupted
    );
}

/// Taint analysis: credential flow through pipelines.
///
/// Each tainted pipeline is rendered as a two-level block:
///   header — timestamp + full redacted command
///   step rows — index, tool, label, raw step text
///
/// Labels are colour-coded:
///   TAINT-SOURCE / CREDENTIAL-USE — yellow  (secret present)
///   PROPAGATED                    — dim     (data flows through)
///   NETWORK-SINK / FILE-SINK /
///   PROCESS-SINK                  — red     (secret reaches output ⚠)
///   DISCARDED                     — green   (taint terminated safely)
///   CLEAN                         — dim grey (not shown unless --all)
pub fn print_taint(pipelines: &[TaintedPipeline], show_clean: bool) {
    if pipelines.is_empty() {
        println!("{}", "=== taint analysis: credential flow ===".bold().cyan());
        println!("{}", "No commands with credential exposure found in corpus.".yellow());
        println!(
            "{}",
            "  (Commands with --token, --password, API_KEY=… etc. will appear here once recorded)"
                .dimmed()
        );
        return;
    }

    let warning_count: usize = pipelines.iter()
        .flat_map(|p| &p.steps)
        .filter(|s| s.label.is_warning())
        .count();

    println!("{}", "=== taint analysis: credential flow ===".bold().cyan());
    println!(
        "{} pipelines  {}",
        pipelines.len().to_string().yellow(),
        if warning_count > 0 {
            format!("{} ⚠  sinks reached", warning_count).red().to_string()
        } else {
            "no sinks reached".dimmed().to_string()
        }
    );
    println!();

    for pipeline in pipelines {
        // Trim ISO timestamp
        let ts = if pipeline.timestamp_iso.len() >= 19 {
            &pipeline.timestamp_iso[..19]
        } else {
            &pipeline.timestamp_iso
        };

        // Header: timestamp + command
        println!(
            "{} {}",
            format!("[{}]", ts).dimmed(),
            pipeline.command_text.bold()
        );

        for step in &pipeline.steps {
            if !show_clean && step.label == TaintLabel::Clean {
                continue;
            }

            let label_str = step.label.as_str();
            let label_cell = match &step.label {
                TaintLabel::Clean         => label_str.dimmed().to_string(),
                TaintLabel::TaintSource   => label_str.yellow().bold().to_string(),
                TaintLabel::CredentialUse => label_str.yellow().to_string(),
                TaintLabel::Propagated    => label_str.dimmed().to_string(),
                TaintLabel::NetworkSink   => format!("{} ⚠", label_str).red().bold().to_string(),
                TaintLabel::FileSink      => format!("{} ⚠", label_str).red().to_string(),
                TaintLabel::ProcessSink   => format!("{} ⚠", label_str).red().to_string(),
                TaintLabel::Discarded     => label_str.green().dimmed().to_string(),
                // Orange: structural concern, not confirmed taint escape
                TaintLabel::ResponseSink  => format!("{} ⚠", label_str).truecolor(255, 165, 0).to_string(),
            };

            // Truncate raw if long
            let raw_display = if step.raw.len() > 60 {
                format!("{}…", &step.raw[..59])
            } else {
                step.raw.clone()
            };

            // Special note for tee: it also writes to file
            let note = if step.tool == "tee"
                && matches!(step.label, TaintLabel::Propagated | TaintLabel::TaintSource)
            {
                "  [tee: also writes to file]".yellow().dimmed().to_string()
            } else {
                String::new()
            };

            println!(
                "  {:>2}  {:<10}  [{:<16}]  {}{}",
                step.step_index,
                step.tool.cyan().to_string(),
                label_cell,
                raw_display.dimmed(),
                note
            );
        }
        println!();
    }
}

/// Terminal table: ranked directed edges with weight bar chart.
pub fn print_graph(edges: &[ToolEdge]) {
    if edges.is_empty() {
        println!("{}", "No tool transitions recorded yet. Run some pipelines first.".yellow());
        return;
    }
    println!("{}", "=== tool transition graph ===".bold().cyan());
    let mut tbl = Table::new();
    tbl.set_header(vec![
        Cell::new("#").add_attribute(Attribute::Bold),
        Cell::new("Weight").add_attribute(Attribute::Bold),
        Cell::new("From").add_attribute(Attribute::Bold),
        Cell::new("Via").add_attribute(Attribute::Bold),
        Cell::new("To").add_attribute(Attribute::Bold),
        Cell::new("").add_attribute(Attribute::Bold),
    ]);
    let max = edges.first().map(|e| e.weight).unwrap_or(1);
    for (i, e) in edges.iter().enumerate() {
        let bar_len = ((e.weight as f64 / max as f64) * 28.0) as usize;
        let connector_cell = match e.connector.as_str() {
            "|"  => Cell::new("|").fg(Color::Green),
            "&&" => Cell::new("&&").fg(Color::Cyan),
            "||" => Cell::new("||").fg(Color::Yellow),
            ";"  => Cell::new(";").fg(Color::DarkGrey),
            _    => Cell::new(&e.connector),
        };
        tbl.add_row(vec![
            Cell::new((i + 1).to_string()),
            Cell::new(e.weight.to_string()).fg(Color::Yellow),
            Cell::new(&e.from).fg(Color::Green),
            connector_cell,
            Cell::new(&e.to).fg(Color::Cyan),
            Cell::new("#".repeat(bar_len)).fg(Color::DarkGrey),
        ]);
    }
    println!("{tbl}");
    println!(
        "  {}  ({} pipe  {} cond  {} fallback  {} seq)",
        format!("{} edges shown", edges.len()).dimmed(),
        "|".green(),
        "&&".cyan(),
        "||".yellow(),
        ";".dimmed()
    );
}

/// Graphviz DOT output — pipe to `dot -Tpng -o graph.png` or `dot -Tsvg`.
///
/// Edge width scales linearly with weight (0.5–6.0 pt).
/// Colors by connector: pipe=green, &&=cyan, ||=orange, ;=grey.
pub fn print_dot(edges: &[ToolEdge]) {
    if edges.is_empty() {
        eprintln!("No tool transitions recorded yet.");
        return;
    }
    let max_weight = edges.iter().map(|e| e.weight).max().unwrap_or(1) as f64;

    println!("digraph tapeworm {{");
    println!("    rankdir=LR;");
    println!("    bgcolor=\"#1e1e1e\";");
    println!("    node [shape=box, style=filled, fillcolor=\"#2d2d2d\",");
    println!("          fontcolor=\"#e0e0e0\", fontname=\"monospace\", fontsize=11];");
    println!("    edge [fontname=\"monospace\", fontsize=9, fontcolor=\"#aaaaaa\"];");
    println!();

    // Collect unique node names
    let mut nodes: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for e in edges {
        nodes.insert(&e.from);
        nodes.insert(&e.to);
    }
    for node in &nodes {
        println!("    \"{node}\";");
    }
    println!();

    for e in edges {
        let penwidth = 0.5 + (e.weight as f64 / max_weight) * 5.5;
        let (color, style) = match e.connector.as_str() {
            "|"  => ("#44bb66", "solid"),
            "&&" => ("#4499dd", "solid"),
            "||" => ("#dd8833", "dashed"),
            ";"  => ("#888888", "dotted"),
            _    => ("#aaaaaa", "solid"),
        };
        println!(
            "    \"{}\" -> \"{}\" [label=\"{}\", penwidth={:.2}, color=\"{}\", style=\"{}\"];",
            e.from, e.to, e.weight, penwidth, color, style
        );
    }
    println!("}}");
}

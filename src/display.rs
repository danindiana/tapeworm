use crate::db::{SessionSummary, ToolEdge};
use crate::record::CommandRecord;
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

    println!("{}", "Activity by hour (UTC):".bold());

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
        Cell::new("ms").add_attribute(Attribute::Bold),
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
        tbl.add_row(vec![
            Cell::new((i + 1).to_string()),
            Cell::new(ts),
            exit_cell,
            Cell::new(r.duration_ms.to_string()),
            Cell::new(cwd_display),
            Cell::new(cmd_display),
        ]);
    }
    println!("{tbl}");
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
        "{}",
        format!(
            "  {} edges shown  (| pipe  {} && conditional  {} || fallback  {} ; sequential)",
            edges.len(),
            "".to_string(), // spacer — colored labels follow
            "".to_string(),
            "".to_string(),
        )
        .dimmed()
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

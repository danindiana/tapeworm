use crate::record::CommandRecord;
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

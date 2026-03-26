mod db;
mod display;
mod embed;
mod parse;
mod record;
mod semantic;
mod shell;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use record::CommandRecord;
use std::env;

#[derive(Parser)]
#[command(
    name = "tapeworm",
    about = "Terminal Activity & Process Execution Workflow Observer/Recorder",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print shell integration snippet — add `eval "$(tapeworm init)"` to .zshrc/.bashrc
    Init {
        /// Shell type: zsh (default) or bash
        #[arg(long, default_value = "zsh")]
        shell: String,
    },

    /// Generate and print a new session UUID (used internally by shell snippets)
    SessionId,

    /// Record a command execution — called automatically by shell hooks
    Record {
        #[arg(long)]
        cmd: String,
        #[arg(long)]
        cwd: String,
        #[arg(long, default_value_t = 0)]
        exit: i64,
        #[arg(long, default_value_t = 0)]
        duration: i64,
        #[arg(long, default_value = "")]
        session: String,
    },

    /// Display recent command history
    Log {
        /// Number of records to show
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },

    /// Search command history by substring pattern
    Search {
        pattern: String,
        #[arg(short, long, default_value_t = 100)]
        limit: usize,
    },

    /// Export all records to stdout
    Export {
        #[arg(long, default_value = "json", value_parser = ["json", "csv"])]
        format: String,
    },

    /// Show usage statistics and activity charts
    Stats,

    /// Show top tools ranked by frequency across all pipeline steps
    Tools {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },

    /// Show top pipeline patterns and most common pipe bigrams (A | B)
    Pipes {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },

    /// Generate Ollama embeddings for unprocessed commands
    Embed {
        /// Ollama embedding model
        #[arg(long, default_value = embed::DEFAULT_MODEL)]
        model: String,
        /// Ollama base URL
        #[arg(long, default_value = embed::DEFAULT_URL)]
        url: String,
        /// Max commands to embed in this run (0 = all pending)
        #[arg(short, long, default_value_t = 0)]
        limit: usize,
    },

    /// Semantic similarity search using stored embeddings
    Semantic {
        /// Natural language query
        query: String,
        /// Number of results to return
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
        /// Ollama base URL
        #[arg(long, default_value = embed::DEFAULT_URL)]
        url: String,
        /// Embedding model (must match what was used during embed)
        #[arg(long, default_value = embed::DEFAULT_MODEL)]
        model: String,
    },

    /// Print path to the SQLite database file
    DbPath,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { shell } => {
            let snippet = match shell.as_str() {
                "bash" => shell::bash_snippet(),
                _ => shell::zsh_snippet(),
            };
            print!("{}", snippet);
        }

        Commands::SessionId => {
            println!("{}", uuid::Uuid::new_v4());
        }

        Commands::Record { cmd, cwd, exit, duration, session } => {
            let shell_name = env::var("TAPEWORM_SHELL").unwrap_or_else(|_| {
                env::var("SHELL")
                    .unwrap_or_default()
                    .rsplit('/')
                    .next()
                    .unwrap_or("unknown")
                    .to_string()
            });
            let user = env::var("USER").unwrap_or_default();
            let hostname = hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_default();

            let r = CommandRecord::new(
                cmd, cwd, exit, duration, shell_name, user, hostname, session,
            );
            let conn = db::open()?;
            let command_id = db::insert(&conn, &r)?;
            let steps = parse::parse_pipeline(&r.command);
            if !steps.is_empty() {
                db::insert_pipeline_steps(&conn, command_id, &steps)?;
            }
        }

        Commands::Log { limit } => {
            let conn = db::open()?;
            let records = db::recent(&conn, limit)?;
            display::print_log(&records);
        }

        Commands::Search { pattern, limit } => {
            let conn = db::open()?;
            let records = db::search(&conn, &pattern, limit)?;
            display::print_log(&records);
        }

        Commands::Export { format } => {
            let conn = db::open()?;
            let records = db::all(&conn)?;
            match format.as_str() {
                "csv" => export_csv(&records)?,
                _ => export_json(&records)?,
            }
        }

        Commands::Stats => {
            let conn = db::open()?;
            let total = db::total_count(&conn)?;
            let avg_ms = db::avg_duration(&conn)?;
            let top = db::top_commands(&conn, 20)?;
            let hourly = db::hourly_distribution(&conn)?;
            display::print_stats(total, avg_ms, &top, &hourly);
        }

        Commands::Tools { limit } => {
            let conn = db::open()?;
            let tools = db::top_tools(&conn, limit)?;
            display::print_tools(&tools);
        }

        Commands::Pipes { limit } => {
            let conn = db::open()?;
            let patterns = db::top_pipelines(&conn, limit)?;
            let bigrams = db::top_bigrams(&conn, limit)?;
            display::print_pipes(&patterns, &bigrams);
        }

        Commands::Embed { model, url, limit } => {
            let client = embed::OllamaClient::new(&url, &model);
            client.check_model().context("checking embedding model")?;

            let conn = db::open()?;
            let pending = db::get_unembedded(&conn, limit)?;
            let total = pending.len();
            if total == 0 {
                println!("All commands already embedded.");
            } else {
                println!("Embedding {} commands with {} …", total, model);
                let mut done = 0usize;
                let mut errors = 0usize;
                for (command_id, command, cwd) in &pending {
                    let text = embed::embed_text(command, cwd);
                    match client.embed(&text) {
                        Ok(vec) => {
                            db::insert_embedding(&conn, *command_id, &model, &vec)?;
                            done += 1;
                            // In-place progress line
                            eprint!("\r  {}/{} embedded", done, total);
                        }
                        Err(e) => {
                            errors += 1;
                            eprintln!("\r  [{}/{}] error embedding cmd {}: {}", done, total, command_id, e);
                        }
                    }
                }
                eprintln!("\r  Done. {} embedded, {} errors.              ", done, errors);
            }
        }

        Commands::Semantic { query, limit, url, model } => {
            let client = embed::OllamaClient::new(&url, &model);
            let query_vec = client.embed(&query)
                .context("embedding query — is Ollama running and model available?")?;

            let conn = db::open()?;
            let corpus = db::get_all_embeddings(&conn)?;
            if corpus.is_empty() {
                anyhow::bail!("No embeddings stored yet. Run `tapeworm embed` first.");
            }

            let matches = semantic::top_k_similar(&query_vec, &corpus, limit);
            let ids: Vec<i64> = matches.iter().map(|(id, _)| *id).collect();
            let mut records = db::get_commands_by_ids(&conn, &ids)?;

            // Re-order records to match similarity ranking
            records.sort_by_key(|r| ids.iter().position(|id| Some(*id) == r.id).unwrap_or(usize::MAX));

            display::print_semantic_results(&records, &matches);
        }

        Commands::DbPath => {
            println!("{}", db::db_path().display());
        }
    }

    Ok(())
}

fn export_json(records: &[CommandRecord]) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(records)?);
    Ok(())
}

fn export_csv(records: &[CommandRecord]) -> Result<()> {
    let mut wtr = csv::Writer::from_writer(std::io::stdout());
    wtr.write_record([
        "id",
        "timestamp_unix",
        "timestamp_iso",
        "command",
        "cwd",
        "exit_code",
        "duration_ms",
        "shell",
        "user",
        "hostname",
        "session_id",
    ])?;
    for r in records {
        wtr.write_record([
            r.id.map(|i| i.to_string()).unwrap_or_default(),
            r.timestamp_unix.to_string(),
            r.timestamp_iso.clone(),
            r.command.clone(),
            r.cwd.clone(),
            r.exit_code.to_string(),
            r.duration_ms.to_string(),
            r.shell.clone(),
            r.user.clone(),
            r.hostname.clone(),
            r.session_id.clone(),
        ])?;
    }
    wtr.flush()?;
    Ok(())
}

mod archetype;
mod config;
mod db;
mod display;
mod embed;
mod parse;
mod record;
mod redact;
mod semantic;
mod shell;
mod taint;
mod timefilter;

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
        /// Include --embed flag in hook (requires Ollama to be reachable at record time)
        #[arg(long)]
        auto_embed: bool,
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
        /// Milliseconds since the previous command finished (idle + think time)
        #[arg(long, default_value_t = 0)]
        gap: i64,
        #[arg(long, default_value = "")]
        session: String,
        /// Also embed this command inline (silently skips if Ollama unavailable)
        #[arg(long)]
        embed: bool,
    },

    /// Display recent command history
    Log {
        /// Number of records to show
        #[arg(short, long)]
        limit: Option<usize>,
        /// Show commands since duration ago: 30m, 2h, 1d, 1w
        #[arg(long)]
        since: Option<String>,
        /// Show commands since midnight today
        #[arg(long)]
        today: bool,
        /// Filter to a specific session ID (or prefix)
        #[arg(long)]
        session: Option<String>,
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

    /// Session intelligence: list sessions, show timeline, failure chains
    Session {
        #[command(subcommand)]
        action: SessionCmd,
    },

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

    /// Show tool transition graph — which tools flow into which, and how often
    Graph {
        /// Minimum edge weight to include (filters noise)
        #[arg(long, default_value_t = 2)]
        min_weight: i64,
        /// Output Graphviz DOT format — pipe to: dot -Tpng -o graph.png
        #[arg(long)]
        dot: bool,
        /// Edge type filter: all (default), pipe (| only), seq (&& / || / ;)
        #[arg(long, default_value = "all")]
        edge_type: String,
        /// Max edges to show
        #[arg(short, long, default_value_t = 60)]
        limit: usize,
    },

    /// Generate Ollama embeddings for unprocessed commands
    Embed {
        /// Ollama embedding model
        #[arg(long)]
        model: Option<String>,
        /// Ollama base URL
        #[arg(long)]
        url: Option<String>,
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
        #[arg(long)]
        url: Option<String>,
        /// Embedding model (must match what was used during embed)
        #[arg(long)]
        model: Option<String>,
    },

    /// Forward taint analysis: trace credential flow through recorded pipelines
    Taint {
        /// Also show clean (untainted) steps in the output
        #[arg(long)]
        all: bool,
    },

    /// Show active configuration path and values
    Config,

    /// Print path to the SQLite database file
    DbPath,
}

#[derive(Subcommand)]
enum SessionCmd {
    /// List recent sessions with summary stats
    List {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Show full command timeline for a session
    Show {
        /// Session ID or unique prefix
        session_id: String,
        #[arg(short, long, default_value_t = 500)]
        limit: usize,
    },
    /// Show commands that ran immediately after a failure (failure chains)
    Failures {
        #[arg(short, long, default_value_t = 30)]
        limit: usize,
    },
    /// Classify sessions by behavioral archetype (burst / debugging / focused / exploratory)
    Archetype {
        #[arg(short, long, default_value_t = 30)]
        limit: usize,
    },
}

fn main() -> Result<()> {
    let cfg = config::load();
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { shell, auto_embed } => {
            // auto_embed flag overrides config if explicitly passed;
            // otherwise respect config.ollama.auto_embed
            let do_embed = auto_embed || cfg.ollama.auto_embed;
            let snippet = match shell.as_str() {
                "bash" => shell::bash_snippet(do_embed),
                _ => shell::zsh_snippet(do_embed),
            };
            print!("{}", snippet);
        }

        Commands::SessionId => {
            println!("{}", uuid::Uuid::new_v4());
        }

        Commands::Record { cmd, cwd, exit, duration, gap, session, embed: do_embed } => {
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

            let cmd = redact::redact_command(&cmd);
            let r = CommandRecord::new(
                cmd, cwd, exit, duration, gap, shell_name, user, hostname, session,
            );
            let conn = db::open()?;
            let command_id = db::insert(&conn, &r)?;
            let steps = parse::parse_pipeline(&r.command);
            if !steps.is_empty() {
                db::insert_pipeline_steps(&conn, command_id, &steps)?;
            }

            // Inline embedding — silently skip on any error so the hook never breaks
            if do_embed || cfg.ollama.auto_embed {
                let url = cfg.ollama.url.clone();
                let model = cfg.ollama.model.clone();
                let text = embed::embed_text(&r.command, &r.cwd);
                let client = embed::OllamaClient::new(&url, &model);
                if let Ok(vec) = client.embed(&text) {
                    let _ = db::insert_embedding(&conn, command_id, &model, &vec);
                }
            }
        }

        Commands::Log { limit, since, today, session } => {
            let conn = db::open()?;
            let lim = limit.unwrap_or(cfg.display.log_limit);

            let records = if let Some(sid) = session {
                db::recent_in_session(&conn, &sid, lim)?
            } else if today {
                let since_ts = timefilter::today_start_unix();
                db::recent_since(&conn, since_ts, lim)?
            } else if let Some(dur) = since {
                let since_ts = timefilter::since_unix(&dur)
                    .context("parsing --since duration")?;
                db::recent_since(&conn, since_ts, lim)?
            } else {
                db::recent(&conn, lim)?
            };
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

        Commands::Session { action } => {
            let conn = db::open()?;
            match action {
                SessionCmd::List { limit } => {
                    let sessions = db::list_sessions(&conn, limit)?;
                    display::print_sessions(&sessions);
                }
                SessionCmd::Show { session_id, limit } => {
                    let records = db::recent_in_session(&conn, &session_id, limit)?;
                    display::print_session_timeline(&session_id, &records);
                }
                SessionCmd::Failures { limit } => {
                    let chains = db::failure_chains(&conn, limit)?;
                    display::print_failure_chains(&chains);
                }
                SessionCmd::Archetype { limit } => {
                    let raw = db::session_raw_stats(&conn, limit)?;
                    let sids: Vec<&str> = raw.iter().map(|s| s.session_id.as_str()).collect();
                    let tool_map = db::session_tool_freqs(&conn, &sids)?;

                    let pairs: Vec<_> = raw.into_iter().map(|s| {
                        let freqs = tool_map.get(&s.session_id)
                            .cloned()
                            .unwrap_or_default();
                        let entropy = archetype::tool_entropy(&freqs);
                        let cv = archetype::gap_cv(s.gap_variance, s.mean_gap_ms);
                        let features = archetype::SessionFeatures {
                            session_id:   s.session_id,
                            start_unix:   s.start_unix,
                            shell:        s.shell,
                            cmd_count:    s.cmd_count,
                            failure_rate: s.failure_rate,
                            mean_gap_ms:  s.mean_gap_ms,
                            max_gap_ms:   s.max_gap_ms,
                            gap_cv:       cv,
                            tool_entropy: entropy,
                        };
                        let classification = archetype::classify(&features);
                        (features, classification)
                    }).collect();

                    display::print_archetypes(&pairs);
                }
            }
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

        Commands::Graph { min_weight, dot, edge_type, limit } => {
            let conn = db::open()?;
            let edges = db::tool_transitions(&conn, &edge_type, min_weight, limit)?;
            if dot {
                display::print_dot(&edges);
            } else {
                display::print_graph(&edges);
            }
        }

        Commands::Embed { model, url, limit } => {
            let model = model.unwrap_or(cfg.ollama.model.clone());
            let url = url.unwrap_or(cfg.ollama.url.clone());
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
            let model = model.unwrap_or(cfg.ollama.model.clone());
            let url = url.unwrap_or(cfg.ollama.url.clone());
            let client = embed::OllamaClient::new(&url, &model);
            let query_vec = client
                .embed(&query)
                .context("embedding query — is Ollama running and model available?")?;

            let conn = db::open()?;
            let corpus = db::get_all_embeddings(&conn)?;
            if corpus.is_empty() {
                anyhow::bail!("No embeddings stored yet. Run `tapeworm embed` first.");
            }

            let matches = semantic::top_k_similar(&query_vec, &corpus, limit);
            let ids: Vec<i64> = matches.iter().map(|(id, _)| *id).collect();
            let mut records = db::get_commands_by_ids(&conn, &ids)?;
            records.sort_by_key(|r| {
                ids.iter().position(|id| Some(*id) == r.id).unwrap_or(usize::MAX)
            });
            display::print_semantic_results(&records, &matches);
        }

        Commands::Taint { all } => {
            let conn = db::open()?;
            let rows = db::tainted_step_rows(&conn)?;
            let pipelines = taint::build_tainted_pipelines(rows);
            display::print_taint(&pipelines, all);
        }

        Commands::Config => {
            let path = config::config_path();
            println!("Config file: {}", path.display());
            if path.exists() {
                println!("{}", std::fs::read_to_string(&path)?);
            } else {
                let created = config::init_default()?;
                println!("Created default config at {}", created.display());
                println!("{}", std::fs::read_to_string(&created)?);
            }
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
        "id", "timestamp_unix", "timestamp_iso", "command", "cwd",
        "exit_code", "duration_ms", "gap_ms", "shell", "user", "hostname", "session_id",
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
            r.gap_ms.to_string(),
            r.shell.clone(),
            r.user.clone(),
            r.hostname.clone(),
            r.session_id.clone(),
        ])?;
    }
    wtr.flush()?;
    Ok(())
}

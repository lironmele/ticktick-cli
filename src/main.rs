use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rusqlite::Connection;
use std::path::Path;

mod commands;

#[derive(Parser)]
#[command(name = "ticktick")]
#[command(version = "0.1.0")]
#[command(about = "TickTick CLI - Read-only interface to your TickTick data", long_about = None)]
struct Cli {
    #[arg(short, long)]
    db: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all projects
    Projects,
    /// List all tasks (optionally filtered by project)
    Tasks {
        /// Filter by project ID (use "inbox" for inbox, or full project ID)
        #[arg(short, long)]
        project: Option<String>,
        /// Show only incomplete tasks
        #[arg(short, long)]
        pending: bool,
        /// Show only completed tasks
        #[arg(short, long)]
        completed: bool,
    },
    /// List all tags
    Tags,
    /// Show task details
    Show {
        /// Task ID (_Id from database)
        id: i64,
    },
}

fn get_default_db_path() -> String {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    format!("{}\\Tick_Tick\\TickTick.db", appdata)
}

fn get_db_path(db_arg: &str) -> Result<String> {
    let path = Path::new(db_arg);
    if path.exists() {
        Ok(db_arg.to_string())
    } else {
        anyhow::bail!("Database file not found: {}", db_arg);
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let db_arg = cli.db.unwrap_or_else(get_default_db_path);
    let db_path = get_db_path(&db_arg)?;

    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open database: {}", db_path))?;

    match &cli.command {
        Commands::Projects => commands::list_projects(&conn),
        Commands::Tasks { project, pending, completed } => {
            commands::list_tasks(&conn, project.as_deref(), *pending, *completed)
        }
        Commands::Tags => commands::list_tags(&conn),
        Commands::Show { id } => commands::show_task(&conn, *id),
    }
}
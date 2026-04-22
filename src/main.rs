use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rusqlite::{Connection, OpenFlags};
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
    /// Manage projects
    Projects {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Manage tasks
    Tasks {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Manage tags
    Tags {
        #[command(subcommand)]
        action: TagAction,
    },
    /// Manage pomodoro/focus records
    Pomos {
        #[command(subcommand)]
        action: PomoAction,
    },
}

#[derive(Subcommand)]
enum ProjectAction {
    /// List all projects
    List,
}

#[derive(Subcommand)]
enum TaskAction {
    /// List tasks (optionally filtered)
    List {
        /// Filter by project ID (use "inbox" for inbox, or full project ID)
        #[arg(short = 'p', long)]
        project: Option<String>,
        /// Show all tasks (including completed and cancelled)
        #[arg(short, long)]
        show_all: bool,
        /// Filter by tags (comma-separated tag names)
        #[arg(short, long)]
        tags: Option<String>,
    },
    /// Show task details
    Show {
        /// Task ID (_Id from database)
        id: i64,
    },
}

#[derive(Subcommand)]
enum TagAction {
    /// List all tags
    List,
}

#[derive(Subcommand)]
enum PomoAction {
    /// List pomodoro/focus records
    List {
        /// Show only records on this date (YYYY-MM-DD)
        #[arg(long)]
        date: Option<String>,
        /// Show records from this date (YYYY-MM-DD)
        #[arg(long)]
        from: Option<String>,
        /// Show records through this date (YYYY-MM-DD)
        #[arg(long)]
        to: Option<String>,
        /// Filter by task ID (_Id from database) or task sid
        #[arg(short, long)]
        task: Option<String>,
        /// Filter by project ID, project sid, project name, or "inbox"
        #[arg(short, long)]
        project: Option<String>,
        /// Show daily totals instead of individual records
        #[arg(short, long)]
        summary: bool,
        /// Maximum rows to show when listing records
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
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

    let conn = Connection::open_with_flags(
        &db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("Failed to open database: {}", db_path))?;

    match &cli.command {
        Commands::Projects { action } => match action {
            ProjectAction::List => commands::list_projects(&conn),
        },
        Commands::Tasks { action } => match action {
            TaskAction::List {
                project,
                show_all,
                tags,
            } => commands::list_tasks(&conn, project.as_deref(), *show_all, tags.as_deref()),
            TaskAction::Show { id } => commands::show_task(&conn, *id),
        },
        Commands::Tags { action } => match action {
            TagAction::List => commands::list_tags(&conn),
        },
        Commands::Pomos { action } => match action {
            PomoAction::List {
                date,
                from,
                to,
                task,
                project,
                summary,
                limit,
            } => commands::list_pomos(
                &conn,
                date.as_deref(),
                from.as_deref(),
                to.as_deref(),
                task.as_deref(),
                project.as_deref(),
                *summary,
                *limit,
            ),
        },
    }
}

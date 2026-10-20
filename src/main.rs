use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod app;
mod ctx;
mod dates;
mod habits;
mod items;
mod misc;
mod model;
mod out;
mod pomos;
mod projects;
mod tags;
mod tasks;

use ctx::{Ctx, OpenOptions};

#[derive(Parser)]
#[command(name = "ticktick-cli", version)]
#[command(about = "Read and edit the TickTick desktop app's local database")]
#[command(
    after_help = "Writes require the TickTick app to be closed (see `app stop`). \
A backup of the database is taken once per day before the first write.\n\
Ids: tasks accept the numeric ID shown in lists, the 24-char sid, a sid prefix, or an exact title. \
Projects accept a name, ID, sid, or \"inbox\"."
)]
struct Cli {
    /// Path to TickTick.db (default: $TICKTICK_DB, then %APPDATA%\Tick_Tick\TickTick.db)
    #[arg(long, global = true, help_heading = "Global options")]
    db: Option<String>,

    /// Machine-readable JSON output
    #[arg(long, global = true, help_heading = "Global options")]
    json: bool,

    /// Run the change in a transaction and roll it back
    #[arg(long, global = true, help_heading = "Global options")]
    dry_run: bool,

    /// Write even though TickTick is running (changes may be overwritten by the app)
    #[arg(long, global = true, help_heading = "Global options")]
    allow_running: bool,

    /// Skip the automatic daily backup before writing
    #[arg(long, global = true, help_heading = "Global options")]
    no_backup: bool,

    /// For writes: close TickTick first and start it again afterwards so it shows the change
    #[arg(long, global = true, help_heading = "Global options")]
    restart_app: bool,

    /// TickTick.exe path used by --restart-app (default: $TICKTICK_EXE or the usual install folders)
    #[arg(long, global = true, help_heading = "Global options")]
    exe: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Commands {
    /// Tasks and notes
    #[command(alias = "task", alias = "t")]
    Tasks {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Checklist items inside a task
    #[command(alias = "item", alias = "checklist")]
    Items {
        #[command(subcommand)]
        action: ItemAction,
    },
    /// Projects (lists)
    #[command(alias = "project", alias = "lists", alias = "p")]
    Projects {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Project folders
    #[command(alias = "folder", alias = "groups")]
    Folders {
        #[command(subcommand)]
        action: FolderAction,
    },
    /// Kanban sections of a project
    #[command(alias = "columns", alias = "section")]
    Sections {
        #[command(subcommand)]
        action: ColumnAction,
    },
    /// Tags
    #[command(alias = "tag")]
    Tags {
        #[command(subcommand)]
        action: TagAction,
    },
    /// Habits and check-ins
    #[command(alias = "habit")]
    Habits {
        #[command(subcommand)]
        action: HabitAction,
    },
    /// Focus / pomodoro records
    #[command(alias = "pomo", alias = "focus")]
    Pomos {
        #[command(subcommand)]
        action: PomoAction,
    },
    /// Trashed tasks
    Trash {
        #[command(subcommand)]
        action: TrashAction,
    },
    /// Overdue, today and upcoming open tasks
    Agenda {
        /// Number of days to show, starting today
        #[arg(long, default_value_t = 7)]
        days: i64,
    },
    /// Search tasks by text (title, description, checklist)
    Search {
        query: String,
        /// Include completed and won't-do tasks
        #[arg(short = 'a', long)]
        all: bool,
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },
    /// Database, user and app status
    Info,
    /// Dump projects, tags and tasks as one JSON document
    Export {
        /// Include completed / won't-do tasks
        #[arg(long)]
        all: bool,
    },
    /// Comments on a task (read-only)
    Comments { task: String },
    /// Saved smart-list filters (read-only)
    Filters,
    /// Run raw SQL (read-only unless --write)
    Sql {
        query: String,
        #[arg(long)]
        write: bool,
    },
    /// Show tables, or one table's definition
    Schema { table: Option<String> },
    /// Copy the database to a backup file
    Backup {
        /// Destination (default: next to the DB with a timestamp)
        #[arg(short, long)]
        out: Option<String>,
    },
    /// Control the TickTick desktop app
    App {
        #[command(subcommand)]
        action: AppAction,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // parsed once per run
enum TaskAction {
    /// List tasks (open tasks by default)
    #[command(alias = "ls")]
    List(tasks::ListArgs),
    /// Show one task with checklist, reminders and subtasks
    #[command(alias = "get")]
    Show { task: String },
    /// Create a task
    #[command(alias = "create", alias = "new")]
    Add(tasks::AddArgs),
    /// Change a task's fields, move it, or re-parent it
    #[command(alias = "update", alias = "set")]
    Edit(tasks::EditArgs),
    /// Complete tasks (a repeating task advances to its next date)
    #[command(alias = "done", alias = "check")]
    Complete {
        #[arg(required = true)]
        tasks: Vec<String>,
        /// For repeating tasks: complete the whole series instead of one occurrence
        #[arg(long)]
        whole_series: bool,
        /// Completion time (default now)
        #[arg(long)]
        at: Option<String>,
    },
    /// Mark tasks as won't do
    #[command(alias = "abandon", alias = "wont-do")]
    Wontdo {
        #[arg(required = true)]
        tasks: Vec<String>,
        #[arg(long)]
        whole_series: bool,
    },
    /// Reopen completed / won't-do tasks
    #[command(alias = "uncomplete", alias = "undo")]
    Reopen {
        #[arg(required = true)]
        tasks: Vec<String>,
    },
    /// Move tasks (and subtasks) to the trash
    #[command(alias = "rm", alias = "trash")]
    Delete {
        #[arg(required = true)]
        tasks: Vec<String>,
        /// Delete permanently instead of trashing
        #[arg(long)]
        forever: bool,
    },
    /// Restore tasks from the trash
    Restore {
        #[arg(required = true)]
        tasks: Vec<String>,
    },
    /// Move tasks to another project
    #[command(alias = "mv")]
    Move {
        #[arg(required = true)]
        tasks: Vec<String>,
        #[arg(short, long)]
        project: String,
    },
    /// Copy a task with its checklist and reminders
    #[command(alias = "copy")]
    Duplicate { task: String },
}

#[derive(Subcommand)]
enum ItemAction {
    /// List a task's checklist
    #[command(alias = "ls")]
    List { task: String },
    /// Add checklist items
    Add {
        task: String,
        #[arg(required = true)]
        titles: Vec<String>,
        /// Insert before this 1-based position instead of appending
        #[arg(long)]
        at: Option<usize>,
    },
    /// Check items (by 1-based position, sid, or title)
    #[command(alias = "complete", alias = "done")]
    Check {
        task: String,
        #[arg(required = true)]
        items: Vec<String>,
        /// Complete the task when every item is checked
        #[arg(long)]
        complete_task: bool,
    },
    /// Uncheck items
    Uncheck {
        task: String,
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Rename or date an item
    Edit {
        task: String,
        item: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        date: Option<String>,
        #[arg(long)]
        clear_date: bool,
    },
    /// Delete items
    #[command(alias = "rm")]
    Delete {
        task: String,
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Turn an item into a subtask
    Promote { task: String, item: String },
}

#[derive(Subcommand)]
enum ProjectAction {
    /// List projects
    #[command(alias = "ls")]
    List {
        /// Include archived projects
        #[arg(short, long)]
        all: bool,
    },
    Show {
        project: String,
    },
    /// Create a project
    #[command(alias = "create")]
    Add {
        name: String,
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        folder: Option<String>,
        /// A note list instead of a task list
        #[arg(long)]
        note: bool,
        #[arg(long, value_enum)]
        view: Option<projects::ViewMode>,
        #[arg(long)]
        muted: bool,
        /// Hide this project's tasks from the All / Today smart lists
        #[arg(long)]
        hide_from_all: bool,
    },
    /// Rename or change project settings
    #[command(alias = "rename", alias = "update")]
    Edit {
        project: String,
        #[arg(long)]
        name: Option<String>,
        /// Hex color, or "none"
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        folder: Option<String>,
        #[arg(long)]
        no_folder: bool,
        #[arg(long, value_enum)]
        view: Option<projects::ViewMode>,
        #[arg(long)]
        muted: Option<bool>,
        #[arg(long)]
        show_in_all: Option<bool>,
        /// task or note
        #[arg(long)]
        kind: Option<String>,
    },
    Archive {
        project: String,
    },
    Unarchive {
        project: String,
    },
    /// Delete a project; its tasks go to the trash unless --move-tasks-to is given
    #[command(alias = "rm")]
    Delete {
        project: String,
        #[arg(long)]
        move_tasks_to: Option<String>,
    },
}

#[derive(Subcommand)]
enum FolderAction {
    #[command(alias = "ls")]
    List,
    Add {
        name: String,
    },
    Rename {
        folder: String,
        name: String,
    },
    /// Delete a folder (its projects are kept, ungrouped)
    #[command(alias = "rm")]
    Delete {
        folder: String,
    },
}

#[derive(Subcommand)]
enum ColumnAction {
    #[command(alias = "ls")]
    List {
        project: String,
    },
    Add {
        project: String,
        name: String,
    },
    Rename {
        project: String,
        section: String,
        name: String,
    },
    /// Delete a section; its tasks move to --move-to or the first other section
    #[command(alias = "rm")]
    Delete {
        project: String,
        section: String,
        #[arg(long)]
        move_to: Option<String>,
    },
}

#[derive(Subcommand)]
enum TagAction {
    #[command(alias = "ls")]
    List,
    Add {
        name: String,
        #[arg(long)]
        color: Option<String>,
        /// Nest under this tag
        #[arg(long)]
        parent: Option<String>,
    },
    /// Rename a tag everywhere
    Rename { tag: String, name: String },
    /// Move all tasks from one tag to another and delete the first
    Merge { from: String, into: String },
    Edit {
        tag: String,
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        no_parent: bool,
    },
    /// Delete a tag and remove it from all tasks
    #[command(alias = "rm")]
    Delete { tag: String },
}

#[derive(Subcommand)]
enum HabitAction {
    #[command(alias = "ls")]
    List {
        /// Include archived habits
        #[arg(short, long)]
        all: bool,
    },
    Show {
        habit: String,
        /// Days of history
        #[arg(long, default_value_t = 30)]
        days: i64,
    },
    /// Check in (boolean habits) or add to the day's value (count habits)
    #[command(alias = "done", alias = "check")]
    Checkin {
        habit: String,
        #[arg(long, allow_hyphen_values = true)]
        date: Option<String>,
        /// Amount to add (count habits); default completes the goal
        #[arg(long)]
        value: Option<f64>,
        /// Set the day's value instead of adding
        #[arg(long)]
        set: bool,
    },
    /// Mark a day as failed
    Fail {
        habit: String,
        #[arg(long, allow_hyphen_values = true)]
        date: Option<String>,
    },
    /// Clear a day's check-in
    #[command(alias = "uncheck")]
    Reset {
        habit: String,
        #[arg(long, allow_hyphen_values = true)]
        date: Option<String>,
    },
    /// Write a note for a habit day
    Note {
        habit: String,
        text: String,
        #[arg(long, allow_hyphen_values = true)]
        date: Option<String>,
    },
    Add {
        name: String,
        /// Daily goal (a goal other than 1 makes a count habit)
        #[arg(long)]
        goal: Option<f64>,
        /// Amount per quick check-in for count habits
        #[arg(long)]
        step: Option<f64>,
        #[arg(long)]
        unit: Option<String>,
        /// daily, weekdays, "every mon,wed", RRULE...
        #[arg(long)]
        repeat: Option<String>,
        #[arg(long)]
        color: Option<String>,
        /// Reminder time HH:MM (repeatable)
        #[arg(long)]
        reminder: Vec<String>,
        #[arg(long)]
        encouragement: Option<String>,
    },
    Edit {
        habit: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        goal: Option<f64>,
        #[arg(long)]
        unit: Option<String>,
        #[arg(long)]
        repeat: Option<String>,
        #[arg(long)]
        encouragement: Option<String>,
    },
    Archive {
        habit: String,
    },
    Unarchive {
        habit: String,
    },
    #[command(alias = "rm")]
    Delete {
        habit: String,
    },
}

#[derive(Subcommand)]
enum PomoAction {
    /// List focus records
    #[command(alias = "ls")]
    List {
        /// Only this date
        #[arg(long, allow_hyphen_values = true)]
        date: Option<String>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(short, long)]
        task: Option<String>,
        #[arg(short, long)]
        project: Option<String>,
        /// Daily totals
        #[arg(short, long)]
        summary: bool,
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },
    /// Log a completed focus session
    Add {
        /// Start time, e.g. "today 9:00"
        #[arg(long)]
        start: String,
        #[arg(long)]
        end: Option<String>,
        /// Length, e.g. 25m (default 25m)
        #[arg(long)]
        duration: Option<String>,
        #[arg(short, long)]
        task: Option<String>,
        /// Record as stopwatch instead of pomodoro
        #[arg(long)]
        stopwatch: bool,
        #[arg(long)]
        note: Option<String>,
    },
    /// Delete a focus record by sid
    #[command(alias = "rm")]
    Delete { sid: String },
}

#[derive(Subcommand)]
enum TrashAction {
    #[command(alias = "ls")]
    List,
    /// Permanently delete everything in the trash
    Empty,
    Restore {
        #[arg(required = true)]
        tasks: Vec<String>,
    },
}

#[derive(Subcommand)]
enum AppAction {
    /// Is TickTick running?
    Status,
    /// Close TickTick (it must be closed before writing)
    Stop {
        /// Only ask it to close; never kill it
        #[arg(long)]
        graceful_only: bool,
        /// Seconds to wait for a graceful exit before killing
        #[arg(long, default_value_t = 8)]
        grace: u64,
    },
    /// Start TickTick so it picks up CLI changes
    Start,
    /// Stop, then start again
    Restart,
}

fn default_db() -> PathBuf {
    if let Ok(p) = std::env::var("TICKTICK_DB") {
        return PathBuf::from(p);
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("Tick_Tick").join("TickTick.db");
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        let mac = home.join("Library/Application Support/Tick_Tick/TickTick.db");
        if mac.exists() {
            return mac;
        }
        return home.join(".local/share/Tick_Tick/TickTick.db");
    }
    PathBuf::from("TickTick.db")
}

fn needs_write(c: &Commands) -> bool {
    use Commands::*;
    match c {
        Tasks { action } => !matches!(action, TaskAction::List(_) | TaskAction::Show { .. }),
        Items { action } => !matches!(action, ItemAction::List { .. }),
        Projects { action } => !matches!(
            action,
            ProjectAction::List { .. } | ProjectAction::Show { .. }
        ),
        Folders { action } => !matches!(action, FolderAction::List),
        Sections { action } => !matches!(action, ColumnAction::List { .. }),
        Tags { action } => !matches!(action, TagAction::List),
        Habits { action } => !matches!(action, HabitAction::List { .. } | HabitAction::Show { .. }),
        Pomos { action } => !matches!(action, PomoAction::List { .. }),
        Trash { action } => !matches!(action, TrashAction::List),
        Sql { write, .. } => *write,
        _ => false,
    }
}

fn main() {
    if let Err(e) = run() {
        let json = std::env::args().any(|a| a == "--json");
        if json {
            println!("{}", serde_json::json!({"error": format!("{e:#}")}));
        } else {
            eprintln!("Error: {e:#}");
        }
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    // Exit quietly when stdout is closed early (e.g. piped into `head`).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .unwrap_or("");
        if msg.contains("Broken pipe") {
            std::process::exit(0);
        }
        default_hook(info);
    }));
    let cli = Cli::parse();
    let db = cli.db.clone().map(PathBuf::from).unwrap_or_else(default_db);
    let exe_arg = cli.exe.clone();

    // Commands that do not need the database.
    match &cli.command {
        Commands::App { action } => return app_cmd(action, cli.json, cli.exe.as_deref()),
        Commands::Backup { out } => return misc::backup(&db, out.as_deref(), cli.json),
        _ => {}
    }

    let write = needs_write(&cli.command);
    let restart = write && cli.restart_app && !cli.dry_run && app::is_running();
    if restart {
        app::stop(false, 8)?;
    }
    let result = run_db(cli, db, write);
    if restart {
        // Start again even if the write failed, so the user gets their app back.
        let started = app::start(exe_arg.as_deref());
        if let Err(e) = started {
            eprintln!("Warning: could not restart TickTick: {e:#}");
        }
    }
    result
}

fn run_db(cli: Cli, db: PathBuf, write: bool) -> Result<()> {
    let opts = OpenOptions {
        write,
        allow_running: cli.allow_running,
        backup: !cli.no_backup,
        json: cli.json,
        dry_run: cli.dry_run,
    };
    let mut ctx = Ctx::open(&db, &opts)?;
    let c = &mut ctx;

    match cli.command {
        Commands::Tasks { action } => match action {
            TaskAction::List(a) => tasks::list(c, &a),
            TaskAction::Show { task } => tasks::show(c, &task),
            TaskAction::Add(a) => tasks::add(c, &a),
            TaskAction::Edit(a) => tasks::edit(c, &a),
            TaskAction::Complete {
                tasks: ts,
                whole_series,
                at,
            } => {
                let at = at
                    .as_deref()
                    .map(dates::parse_when)
                    .transpose()?
                    .map(|w| w.dt);
                tasks::set_status(c, &ts, 2, tasks::CloseOpts { whole_series, at })
            }
            TaskAction::Wontdo {
                tasks: ts,
                whole_series,
            } => tasks::set_status(
                c,
                &ts,
                -1,
                tasks::CloseOpts {
                    whole_series,
                    at: None,
                },
            ),
            TaskAction::Reopen { tasks: ts } => tasks::set_status(
                c,
                &ts,
                0,
                tasks::CloseOpts {
                    whole_series: true,
                    at: None,
                },
            ),
            TaskAction::Delete { tasks: ts, forever } => tasks::trash(c, &ts, forever),
            TaskAction::Restore { tasks: ts } => tasks::restore(c, &ts),
            TaskAction::Move { tasks: ts, project } => tasks::move_many(c, &ts, &project),
            TaskAction::Duplicate { task } => tasks::duplicate(c, &task),
        },
        Commands::Items { action } => match action {
            ItemAction::List { task } => items::list(c, &task),
            ItemAction::Add { task, titles, at } => items::add(c, &task, &titles, at),
            ItemAction::Check {
                task,
                items: is,
                complete_task,
            } => items::set_done(c, &task, &is, true, complete_task),
            ItemAction::Uncheck { task, items: is } => items::set_done(c, &task, &is, false, false),
            ItemAction::Edit {
                task,
                item,
                title,
                date,
                clear_date,
            } => items::edit(
                c,
                &task,
                &item,
                title.as_deref(),
                date.as_deref(),
                clear_date,
            ),
            ItemAction::Delete { task, items: is } => items::delete(c, &task, &is),
            ItemAction::Promote { task, item } => items::promote(c, &task, &item),
        },
        Commands::Projects { action } => match action {
            ProjectAction::List { all } => projects::list(c, all),
            ProjectAction::Show { project } => projects::show(c, &project),
            ProjectAction::Add {
                name,
                color,
                folder,
                note,
                view,
                muted,
                hide_from_all,
            } => projects::add(
                c,
                &name,
                color.as_deref(),
                folder.as_deref(),
                note,
                view,
                muted,
                hide_from_all,
            ),
            ProjectAction::Edit {
                project,
                name,
                color,
                folder,
                no_folder,
                view,
                muted,
                show_in_all,
                kind,
            } => projects::edit(
                c,
                &project,
                name.as_deref(),
                color.as_deref(),
                folder.as_deref(),
                no_folder,
                view,
                muted,
                show_in_all,
                kind.as_deref(),
            ),
            ProjectAction::Archive { project } => projects::set_archived(c, &project, true),
            ProjectAction::Unarchive { project } => projects::set_archived(c, &project, false),
            ProjectAction::Delete {
                project,
                move_tasks_to,
            } => projects::delete(c, &project, move_tasks_to.as_deref()),
        },
        Commands::Folders { action } => match action {
            FolderAction::List => projects::folders_list(c),
            FolderAction::Add { name } => projects::folders_add(c, &name),
            FolderAction::Rename { folder, name } => projects::folders_rename(c, &folder, &name),
            FolderAction::Delete { folder } => projects::folders_delete(c, &folder),
        },
        Commands::Sections { action } => match action {
            ColumnAction::List { project } => projects::columns_list(c, &project),
            ColumnAction::Add { project, name } => projects::columns_add(c, &project, &name),
            ColumnAction::Rename {
                project,
                section,
                name,
            } => projects::columns_rename(c, &project, &section, &name),
            ColumnAction::Delete {
                project,
                section,
                move_to,
            } => projects::columns_delete(c, &project, &section, move_to.as_deref()),
        },
        Commands::Tags { action } => match action {
            TagAction::List => tags::list(c),
            TagAction::Add {
                name,
                color,
                parent,
            } => tags::add(c, &name, color.as_deref(), parent.as_deref()),
            TagAction::Rename { tag, name } => tags::rename(c, &tag, &name),
            TagAction::Merge { from, into } => tags::merge(c, &from, &into),
            TagAction::Edit {
                tag,
                color,
                parent,
                no_parent,
            } => tags::edit(c, &tag, color.as_deref(), parent.as_deref(), no_parent),
            TagAction::Delete { tag } => tags::delete(c, &tag),
        },
        Commands::Habits { action } => match action {
            HabitAction::List { all } => habits::list(c, all),
            HabitAction::Show { habit, days } => habits::show(c, &habit, days),
            HabitAction::Checkin {
                habit,
                date,
                value,
                set,
            } => habits::checkin(
                c,
                &habit,
                date.as_deref(),
                value,
                set,
                habits::CheckMode::Done,
            ),
            HabitAction::Fail { habit, date } => habits::checkin(
                c,
                &habit,
                date.as_deref(),
                None,
                false,
                habits::CheckMode::Fail,
            ),
            HabitAction::Reset { habit, date } => habits::checkin(
                c,
                &habit,
                date.as_deref(),
                None,
                false,
                habits::CheckMode::Reset,
            ),
            HabitAction::Note { habit, text, date } => {
                habits::note(c, &habit, date.as_deref(), &text)
            }
            HabitAction::Add {
                name,
                goal,
                step,
                unit,
                repeat,
                color,
                reminder,
                encouragement,
            } => habits::add(
                c,
                &name,
                goal,
                step,
                unit.as_deref(),
                repeat.as_deref(),
                color.as_deref(),
                &reminder,
                encouragement.as_deref(),
            ),
            HabitAction::Edit {
                habit,
                name,
                goal,
                unit,
                repeat,
                encouragement,
            } => habits::edit(
                c,
                &habit,
                name.as_deref(),
                goal,
                unit.as_deref(),
                repeat.as_deref(),
                encouragement.as_deref(),
            ),
            HabitAction::Archive { habit } => habits::set_archived(c, &habit, true),
            HabitAction::Unarchive { habit } => habits::set_archived(c, &habit, false),
            HabitAction::Delete { habit } => habits::delete(c, &habit),
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
            } => pomos::list(
                c,
                date.as_deref(),
                from.as_deref(),
                to.as_deref(),
                task.as_deref(),
                project.as_deref(),
                summary,
                limit,
            ),
            PomoAction::Add {
                start,
                end,
                duration,
                task,
                stopwatch,
                note,
            } => pomos::add(
                c,
                &start,
                end.as_deref(),
                duration.as_deref(),
                task.as_deref(),
                stopwatch,
                note.as_deref(),
            ),
            PomoAction::Delete { sid } => pomos::delete(c, &sid),
        },
        Commands::Trash { action } => match action {
            TrashAction::List => tasks::trash_list(c),
            TrashAction::Empty => tasks::trash_empty(c),
            TrashAction::Restore { tasks: ts } => tasks::restore(c, &ts),
        },
        Commands::Agenda { days } => misc::agenda(c, days),
        Commands::Search { query, all, limit } => tasks::list(
            c,
            &tasks::ListArgs {
                search: Some(query),
                show_all: all,
                limit,
                sort: tasks::SortBy::Modified,
                ..Default::default()
            },
        ),
        Commands::Info => misc::info(c),
        Commands::Export { all } => misc::export(c, all),
        Commands::Comments { task } => misc::comments(c, &task),
        Commands::Filters => misc::filters(c),
        Commands::Sql { query, write } => misc::sql(c, &query, write),
        Commands::Schema { table } => misc::schema(c, table.as_deref()),
        Commands::App { .. } | Commands::Backup { .. } => unreachable!(),
    }
}

fn app_cmd(action: &AppAction, json: bool, exe: Option<&str>) -> Result<()> {
    match action {
        AppAction::Status => {
            let pids = app::pids();
            out::done(
                json,
                serde_json::json!({"running": !pids.is_empty(), "pids": pids}),
                if pids.is_empty() {
                    "TickTick is not running"
                } else {
                    "TickTick is running"
                },
            );
        }
        AppAction::Stop {
            graceful_only,
            grace,
        } => {
            let stopped = app::stop(*graceful_only, *grace)?;
            out::done(
                json,
                serde_json::json!({"stopped": stopped}),
                if stopped {
                    "TickTick stopped"
                } else {
                    "TickTick was not running"
                },
            );
        }
        AppAction::Start => {
            if app::is_running() {
                bail!("TickTick is already running");
            }
            let p = app::start(exe)?;
            out::done(
                json,
                serde_json::json!({"started": p.display().to_string()}),
                &format!("Started {}", p.display()),
            );
        }
        AppAction::Restart => {
            app::stop(false, 8)?;
            let p = app::start(exe)?;
            out::done(
                json,
                serde_json::json!({"restarted": p.display().to_string()}),
                &format!("Restarted {}", p.display()),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}

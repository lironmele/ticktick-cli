//! Task commands. Writes mirror ticktick_WPF.Service.TaskService so the desktop
//! app shows CLI-made changes exactly like its own after a restart.

use crate::ctx::{self, Ctx, SORT_STEP, WriteEnv, sync};
use crate::dates::{self, Rrule, When};
use crate::model::{self, TASK_COLS, Task};
use crate::out;
use anyhow::{Result, anyhow, bail};
use chrono::{Duration, NaiveDate, NaiveDateTime};
use clap::{Args, ValueEnum};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// list / show / search
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq)]
pub enum StatusFilter {
    Open,
    Completed,
    Wontdo,
    Closed,
    All,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Default)]
pub enum SortBy {
    /// Project order (the app's manual order)
    #[default]
    Order,
    Date,
    Priority,
    Created,
    Modified,
    Completed,
    Title,
}

#[derive(Args, Debug, Default)]
pub struct ListArgs {
    /// Project (name, id, sid, or "inbox"); repeatable
    #[arg(short, long)]
    pub project: Vec<String>,
    /// Only tasks having this tag; repeatable (any match). Use "none" for untagged
    #[arg(short, long)]
    pub tag: Vec<String>,
    /// Require every --tag instead of any
    #[arg(long)]
    pub all_tags: bool,
    /// Status filter
    #[arg(short, long, value_enum)]
    pub status: Option<StatusFilter>,
    /// Shortcut for --status all
    #[arg(short = 'a', long)]
    pub show_all: bool,
    /// Date filter: today, tomorrow, overdue, week (next 7 days), nodate, or a date
    #[arg(short, long, allow_hyphen_values = true)]
    pub due: Option<String>,
    /// Tasks dated on/after this date
    #[arg(long, allow_hyphen_values = true)]
    pub from: Option<String>,
    /// Tasks dated on/before this date
    #[arg(long, allow_hyphen_values = true)]
    pub to: Option<String>,
    /// Minimum priority (low, medium, high)
    #[arg(long)]
    pub priority: Option<String>,
    /// Kind: task, note, checklist
    #[arg(long)]
    pub kind: Option<String>,
    /// Subtasks of this task
    #[arg(long)]
    pub parent: Option<String>,
    /// Only top-level tasks (no subtasks)
    #[arg(long)]
    pub top_level: bool,
    /// Only pinned tasks
    #[arg(long)]
    pub pinned: bool,
    /// Only repeating tasks
    #[arg(long)]
    pub repeating: bool,
    /// Text in title, content or checklist items (case-insensitive)
    #[arg(short = 'q', long)]
    pub search: Option<String>,
    /// Completed on/after this date (with completed/closed status)
    #[arg(long, allow_hyphen_values = true)]
    pub completed_from: Option<String>,
    /// Completed on/before this date
    #[arg(long, allow_hyphen_values = true)]
    pub completed_to: Option<String>,
    /// Modified since this date/time
    #[arg(long, allow_hyphen_values = true)]
    pub modified_since: Option<String>,
    /// Include archived projects
    #[arg(long)]
    pub include_archived: bool,
    /// Sort order
    #[arg(long, value_enum, default_value = "order")]
    pub sort: SortBy,
    /// Reverse sort
    #[arg(long)]
    pub reverse: bool,
    /// Max rows (0 = unlimited)
    #[arg(short, long, default_value_t = 0)]
    pub limit: usize,
}

fn task_date(t: &Task) -> Option<NaiveDateTime> {
    t.start
        .as_deref()
        .or(t.due.as_deref())
        .and_then(dates::parse_db_dt)
}

/// The last day a task covers (all-day spans store an exclusive end).
fn task_end_date(t: &Task) -> Option<NaiveDate> {
    let end = t.due.as_deref().and_then(dates::parse_db_dt)?;
    let start = task_date(t)?;
    if t.all_day && end > start {
        Some((end - Duration::seconds(1)).date())
    } else {
        Some(end.date())
    }
}

pub fn query(ctx: &Ctx, a: &ListArgs) -> Result<Vec<Task>> {
    let conn = &ctx.conn;
    let mut sql = format!(
        "SELECT {TASK_COLS} FROM TaskModel WHERE (deleted = 0 OR deleted IS NULL)
         AND (userId = ? OR userId IS NULL OR userId = '')"
    );
    let mut p: Vec<rusqlite::types::Value> = vec![ctx.user_id.clone().into()];

    let status = if a.show_all {
        StatusFilter::All
    } else {
        a.status.unwrap_or(StatusFilter::Open)
    };
    match status {
        StatusFilter::Open => sql.push_str(" AND status = 0"),
        StatusFilter::Completed => sql.push_str(" AND status IN (1, 2)"),
        StatusFilter::Wontdo => sql.push_str(" AND status = -1"),
        StatusFilter::Closed => sql.push_str(" AND status <> 0"),
        StatusFilter::All => {}
    }
    if !a.project.is_empty() {
        let ids: Vec<String> = a
            .project
            .iter()
            .map(|k| model::resolve_project(conn, &ctx.inbox_id, k).map(|p| p.sid))
            .collect::<Result<_>>()?;
        sql.push_str(&format!(
            " AND projectId IN ({})",
            vec!["?"; ids.len()].join(",")
        ));
        p.extend(ids.into_iter().map(Into::into));
    } else if !a.include_archived {
        sql.push_str(
            " AND projectId NOT IN (SELECT id FROM ProjectModel WHERE closed = 1 OR delete_status = 1)",
        );
    }
    if let Some(parent) = &a.parent {
        let t = model::resolve_task(conn, &ctx.user_id, parent)?;
        sql.push_str(" AND parentId = ?");
        p.push(t.sid.into());
    }
    if a.top_level {
        sql.push_str(" AND (parentId IS NULL OR parentId = '')");
    }
    if let Some(k) = &a.kind {
        let k = match k.to_ascii_lowercase().as_str() {
            "task" | "text" => "TEXT",
            "note" => "NOTE",
            "checklist" => "CHECKLIST",
            other => bail!("unknown kind '{other}'"),
        };
        if k == "TEXT" {
            sql.push_str(" AND (kind = 'TEXT' OR kind IS NULL OR kind = '')");
        } else {
            sql.push_str(" AND kind = ?");
            p.push(k.to_string().into());
        }
    }
    if let Some(pr) = &a.priority {
        sql.push_str(" AND priority >= ?");
        p.push(model::parse_priority(pr)?.into());
    }
    if a.pinned {
        sql.push_str(" AND pinnedTime IS NOT NULL AND pinnedTime NOT IN ('', '0', '-1')");
    }
    if a.repeating {
        sql.push_str(" AND repeatFlag IS NOT NULL AND repeatFlag <> ''");
    }
    if let Some(since) = &a.modified_since {
        sql.push_str(" AND modifiedTime >= ?");
        p.push(ctx::fmt_dt(&dates::parse_when(since)?.dt).into());
    }
    if let Some(f) = &a.completed_from {
        sql.push_str(" AND date(completedTime) >= ?");
        p.push(dates::parse_date(f)?.to_string().into());
    }
    if let Some(t) = &a.completed_to {
        sql.push_str(" AND date(completedTime) <= ?");
        p.push(dates::parse_date(t)?.to_string().into());
    }

    let mut stmt = conn.prepare(&sql)?;
    let mut tasks: Vec<Task> = stmt
        .query_map(rusqlite::params_from_iter(p.iter()), Task::from_row)?
        .collect::<rusqlite::Result<_>>()?;

    // Tag filter
    if !a.tag.is_empty() {
        let wanted: Vec<String> = a
            .tag
            .iter()
            .map(|t| t.trim_start_matches('#').to_lowercase())
            .collect();
        tasks.retain(|t| {
            let have: Vec<String> = t.tags.iter().map(|x| x.to_lowercase()).collect();
            let hit = |w: &String| {
                if w == "none" {
                    have.is_empty()
                } else {
                    // nested tags: "work" also matches children stored as "work/child"? TickTick stores plain names.
                    have.contains(w)
                }
            };
            if a.all_tags {
                wanted.iter().all(hit)
            } else {
                wanted.iter().any(hit)
            }
        });
    }

    // Date filters
    let today = dates::today();
    let range: Option<(Option<NaiveDate>, Option<NaiveDate>)> =
        match a.due.as_deref().map(str::to_ascii_lowercase) {
            None => None,
            Some(d) => Some(match d.as_str() {
                "nodate" | "none" | "no-date" => {
                    tasks.retain(|t| task_date(t).is_none());
                    (None, None)
                }
                "overdue" => {
                    tasks.retain(|t| {
                        let Some(start) = task_date(t) else {
                            return false;
                        };
                        let end_day = task_end_date(t).unwrap_or(start.date());
                        if t.all_day || start.time() == chrono::NaiveTime::MIN {
                            end_day < today
                        } else {
                            let end = t
                                .due
                                .as_deref()
                                .and_then(dates::parse_db_dt)
                                .unwrap_or(start);
                            end < ctx::now_local()
                        }
                    });
                    (None, None)
                }
                "today" => (Some(today), Some(today)),
                "tomorrow" => (
                    Some(today + Duration::days(1)),
                    Some(today + Duration::days(1)),
                ),
                "week" | "7days" | "next7days" => (Some(today), Some(today + Duration::days(6))),
                other => {
                    let d = dates::parse_date(other)?;
                    (Some(d), Some(d))
                }
            }),
        };
    let from = match &a.from {
        Some(f) => Some(dates::parse_date(f)?),
        None => range.and_then(|r| r.0),
    };
    let to = match &a.to {
        Some(t) => Some(dates::parse_date(t)?),
        None => range.and_then(|r| r.1),
    };
    if from.is_some() || to.is_some() {
        let include_overdue_today = matches!(a.due.as_deref(), Some("today"));
        tasks.retain(|t| {
            let Some(start) = task_date(t) else {
                return false;
            };
            let s = start.date();
            let e = task_end_date(t).unwrap_or(s).max(s);
            if include_overdue_today && t.status == 0 && e < today {
                // The app's Today view also lists overdue open tasks.
                return true;
            }
            let after_from = from.is_none_or(|f| e >= f);
            let before_to = to.is_none_or(|x| s <= x);
            after_from && before_to
        });
    }

    if let Some(q) = &a.search {
        let q = q.to_lowercase();
        let item_hits: std::collections::HashSet<String> = {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT TaskServerId FROM TaskDetailItemModel WHERE lower(title) LIKE ?",
            )?;
            stmt.query_map([format!("%{q}%")], |r| r.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect()
        };
        tasks.retain(|t| {
            t.title.to_lowercase().contains(&q)
                || t.content.to_lowercase().contains(&q)
                || t.desc.to_lowercase().contains(&q)
                || item_hits.contains(&t.sid)
        });
    }

    let key_date = |t: &Task| {
        task_date(t)
            .map(|d| d.and_utc().timestamp())
            .unwrap_or(i64::MAX)
    };
    match a.sort {
        SortBy::Order => tasks.sort_by(|x, y| {
            (x.project_id.as_str(), x.sort_order).cmp(&(y.project_id.as_str(), y.sort_order))
        }),
        SortBy::Date => tasks.sort_by_key(|t| (key_date(t), -t.priority)),
        SortBy::Priority => tasks.sort_by_key(|t| (-t.priority, key_date(t))),
        SortBy::Created => tasks.sort_by(|x, y| y.created.cmp(&x.created)),
        SortBy::Modified => tasks.sort_by(|x, y| y.modified.cmp(&x.modified)),
        SortBy::Completed => tasks.sort_by(|x, y| y.completed.cmp(&x.completed)),
        SortBy::Title => tasks.sort_by_key(|t| t.title.to_lowercase()),
    }
    if a.reverse {
        tasks.reverse();
    }
    if a.limit > 0 {
        tasks.truncate(a.limit);
    }
    Ok(tasks)
}

pub fn list(ctx: &Ctx, a: &ListArgs) -> Result<()> {
    let tasks = query(ctx, a)?;
    let rows: Vec<Value> = tasks
        .iter()
        .map(|t| model::task_json(&ctx.conn, &ctx.inbox_id, t, false))
        .collect();
    if ctx.json {
        out::json(&json!(rows));
        return Ok(());
    }
    let table: Vec<Vec<String>> = rows
        .iter()
        .zip(tasks.iter())
        .map(|(r, t)| {
            let st = match t.status {
                0 => "[ ]",
                -1 => "[-]",
                _ => "[x]",
            };
            let mut title = t.title.clone();
            if t.parent_id.as_deref().is_some_and(|p| !p.is_empty()) {
                title = format!("  ↳ {title}");
            }
            if t.is_repeating() {
                title.push_str(" ↻");
            }
            vec![
                t.local_id.to_string(),
                st.into(),
                model::priority_name(t.priority).into(),
                title,
                out::s(&r["project"]),
                out::short_dt(&r["start"], t.all_day),
                t.tags.join(", "),
            ]
        })
        .collect();
    out::table(
        &[
            "ID", "Status", "Priority", "Title", "Project", "Date", "Tags",
        ],
        table,
        "No tasks found.",
    );
    Ok(())
}

pub fn show(ctx: &Ctx, key: &str) -> Result<()> {
    let t = model::resolve_task(&ctx.conn, &ctx.user_id, key)?;
    let v = model::task_json(&ctx.conn, &ctx.inbox_id, &t, true);
    if ctx.json {
        out::json(&v);
        return Ok(());
    }
    println!("{}", t.title);
    println!("{}", "=".repeat(60));
    let line = |k: &str, val: String| {
        if !val.is_empty() {
            println!("{k:<12}{val}");
        }
    };
    line("ID:", format!("{}  (sid {})", t.local_id, t.sid));
    line("Project:", out::s(&v["project"]));
    line("Status:", model::status_name(t.status).into());
    line("Priority:", model::priority_name(t.priority).into());
    line("Kind:", model::kind_name(&t.kind).into());
    line("Date:", out::short_dt(&v["start"], t.all_day));
    line("End:", out::short_dt(&v["due"], t.all_day));
    if t.all_day {
        line("All-day:", "yes".into());
    }
    line("Repeat:", out::s(&v["repeat"]));
    line(
        "Reminders:",
        model::task_reminders(&ctx.conn, &t.sid)?
            .iter()
            .map(|r| model::describe_trigger(r))
            .collect::<Vec<_>>()
            .join(", "),
    );
    line("Tags:", t.tags.join(", "));
    line("Column:", out::s(&v["column"]));
    if let Some(p) = t.parent_id.as_deref().filter(|p| !p.is_empty()) {
        let title = Task::load_by_sid(&ctx.conn, p)?
            .map(|x| x.title)
            .unwrap_or_default();
        line("Parent:", format!("{title} ({p})"));
    }
    if t.pinned.is_some() {
        line("Pinned:", "yes".into());
    }
    if t.deleted != 0 {
        line("Trashed:", "yes".into());
    }
    line("Created:", out::s(&v["created_time"]));
    line("Modified:", out::s(&v["modified_time"]));
    line("Completed:", out::s(&v["completed_time"]));
    let content = out::s(&v["content"]);
    if !content.trim().is_empty() {
        println!("\n{}", content.trim_end());
    }
    if let Some(items) = v["items"].as_array().filter(|i| !i.is_empty()) {
        println!("\nChecklist ({}%):", t.progress);
        for (i, it) in items.iter().enumerate() {
            let mark = if it["done"].as_bool().unwrap_or(false) {
                "[x]"
            } else {
                "[ ]"
            };
            println!("  {}. {mark} {}", i + 1, out::s(&it["title"]));
        }
    }
    if let Some(subs) = v["subtasks"].as_array().filter(|s| !s.is_empty()) {
        println!("\nSubtasks:");
        for s in subs {
            println!(
                "  {} [{}] {}",
                out::s(&s["id"]),
                out::s(&s["status"]),
                out::s(&s["title"])
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared write helpers
// ---------------------------------------------------------------------------

/// Sort order for a new task: above (default) or below the existing siblings.
pub fn new_sort_order(
    conn: &Connection,
    project: &str,
    parent: Option<&str>,
    top: bool,
) -> Result<i64> {
    let (min, max): (Option<i64>, Option<i64>) = conn.query_row(
        "SELECT MIN(sortOrder), MAX(sortOrder) FROM TaskModel
         WHERE projectId = ? AND COALESCE(parentId, '') = ? AND (deleted = 0 OR deleted IS NULL)",
        params![project, parent.unwrap_or("")],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(match (top, min, max) {
        (_, None, _) | (_, _, None) => 0,
        (true, Some(m), _) => {
            let v = m - SORT_STEP;
            if v == 0 { -1 } else { v }
        }
        (false, _, Some(m)) => {
            let v = m + SORT_STEP;
            if v == 0 { 1 } else { v }
        }
    })
}

/// Default "add to top" from the user's TaskDefaultModel (AddTo 0 = top).
fn default_add_to_top(conn: &Connection, user: &str) -> bool {
    conn.query_row(
        "SELECT AddTo FROM TaskDefaultModel WHERE UserId = ? LIMIT 1",
        [user],
        |r| r.get::<_, Option<i64>>(0),
    )
    .optional()
    .ok()
    .flatten()
    .flatten()
    .unwrap_or(0)
        == 0
}

fn default_column(conn: &Connection, project: &str) -> Result<Option<String>> {
    let kanban: bool = conn
        .query_row(
            "SELECT viewMode FROM ProjectModel WHERE id = ?",
            [project],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .is_some_and(|v| v == "kanban");
    if !kanban {
        return Ok(None);
    }
    Ok(conn
        .query_row(
            "SELECT id FROM ColumnModel WHERE projectId = ? AND (deleted = 0 OR deleted IS NULL)
             ORDER BY sortOrder LIMIT 1",
            [project],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn resolve_column(conn: &Connection, project: &str, key: &str) -> Result<String> {
    let cols: Vec<(i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT _Id, id, name FROM ColumnModel WHERE projectId = ? AND (deleted = 0 OR deleted IS NULL)",
        )?;
        stmt.query_map([project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let lower = key.to_lowercase();
    cols.iter()
        .find(|(l, s, n)| s == key || l.to_string() == key || n.to_lowercase() == lower)
        .map(|c| c.1.clone())
        .ok_or_else(|| anyhow!("Section '{key}' not found in this project (see `columns list`)"))
}

/// Normalise tag names the way TagSerializer does and create missing TagModel rows.
pub fn ensure_tags(conn: &Connection, env: &WriteEnv, tags: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for raw in tags {
        let label = raw.replace(['#', '＃'], "").trim().to_string();
        if label.is_empty() {
            continue;
        }
        if label
            .chars()
            .any(|c| "\\/\"'*?<>|:,;".contains(c) || c.is_whitespace())
        {
            bail!("Invalid tag name '{label}' (no spaces or \\ / \" ' * ? < > | : , ;)");
        }
        let name = label.to_lowercase();
        if !out.contains(&name) {
            out.push(name.clone());
        }
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM TagModel WHERE name = ? AND (deleted = 0 OR deleted IS NULL) AND (status IS NULL OR status <> -1)",
                [&name],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            crate::tags::insert_tag(conn, env, &label, None, None)?;
        }
    }
    Ok(out)
}

fn tags_json(tags: &[String]) -> Option<String> {
    if tags.is_empty() {
        None
    } else {
        Some(serde_json::to_string(tags).unwrap())
    }
}

fn touch(conn: &Connection, env: &WriteEnv, sid: &str) -> Result<()> {
    conn.execute(
        "UPDATE TaskModel SET modifiedTime = ? WHERE id = ?",
        params![ctx::now_str(), sid],
    )?;
    ctx::add_sync_status(conn, env, sid, sync::MODIFY)
}

fn set_reminders(conn: &Connection, task: &Task, triggers: &[String]) -> Result<()> {
    conn.execute(
        "DELETE FROM TaskReminderModel WHERE taskserverid = ?",
        [&task.sid],
    )?;
    for t in triggers {
        conn.execute(
            "INSERT INTO TaskReminderModel (id, Taskid, taskserverid, trigger) VALUES (?, ?, ?, ?)",
            params![ctx::new_object_id(), task.local_id, task.sid, t],
        )?;
    }
    conn.execute(
        "UPDATE TaskModel SET reminderCount = ?, reminder = NULL WHERE id = ?",
        params![triggers.len() as i64, task.sid],
    )?;
    Ok(())
}

fn add_child_id(conn: &Connection, parent: &str, child: &str, add: bool) -> Result<()> {
    let current: Option<String> = conn
        .query_row(
            "SELECT childrenString FROM TaskModel WHERE id = ?",
            [parent],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let mut list = model::parse_json_list(current.as_deref());
    if add {
        if !list.iter().any(|c| c == child) {
            list.push(child.to_string());
        }
    } else {
        list.retain(|c| c != child);
    }
    let v = if list.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&list)?)
    };
    conn.execute(
        "UPDATE TaskModel SET childrenString = ? WHERE id = ?",
        params![v, parent],
    )?;
    Ok(())
}

fn descendants(conn: &Connection, sid: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut frontier = vec![sid.to_string()];
    while let Some(cur) = frontier.pop() {
        let mut stmt = conn.prepare(
            "SELECT id FROM TaskModel WHERE parentId = ? AND (deleted = 0 OR deleted IS NULL)",
        )?;
        let kids: Vec<String> = stmt
            .query_map([&cur], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for k in kids {
            if !out.contains(&k) && k != sid {
                out.push(k.clone());
                frontier.push(k);
            }
        }
    }
    Ok(out)
}

/// Dates for a task from --date/--end/--all-day/--time-of-day.
struct DateSet {
    start: Option<String>,
    due: Option<String>,
    all_day: Option<bool>,
}

fn build_dates(
    start: Option<When>,
    end: Option<When>,
    all_day: Option<bool>,
    duration: Option<&str>,
) -> Result<DateSet> {
    let Some(s) = start else {
        if end.is_some() || duration.is_some() {
            bail!("--end/--duration need --date");
        }
        return Ok(DateSet {
            start: None,
            due: None,
            all_day: None,
        });
    };
    let all_day = all_day.unwrap_or(!s.has_time);
    let mut start_dt = s.dt;
    if all_day {
        start_dt = start_dt.date().and_hms_opt(0, 0, 0).unwrap();
    }
    let mut due = None;
    if let Some(e) = end {
        let mut e_dt = e.dt;
        if all_day {
            // inclusive end date -> exclusive midnight after it
            e_dt = e_dt.date().and_hms_opt(0, 0, 0).unwrap() + Duration::days(1);
        } else if !e.has_time {
            e_dt = e.dt.date().and_time(s.dt.time());
        }
        if e_dt < start_dt {
            bail!("--end is before --date");
        }
        due = Some(e_dt);
    } else if let Some(d) = duration {
        let d = dates::parse_duration(d)?;
        due = Some(start_dt + d);
    }
    // A single all-day day has no end date in the app's model.
    if all_day && due == Some(start_dt + Duration::days(1)) {
        due = None;
    }
    Ok(DateSet {
        start: Some(ctx::fmt_dt(&start_dt)),
        due: due.map(|d| ctx::fmt_dt(&d)),
        all_day: Some(all_day),
    })
}

fn pinned_value() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f+0000")
        .to_string()
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Task title
    pub title: String,
    /// Project (name, id, sid, or inbox). Defaults to the parent's project or the inbox
    #[arg(short, long)]
    pub project: Option<String>,
    /// Description / note body
    #[arg(short, long)]
    pub content: Option<String>,
    /// Priority: none, low, medium, high
    #[arg(long)]
    pub priority: Option<String>,
    /// Tag (repeatable or comma-separated); created if missing
    #[arg(short, long, value_delimiter = ',')]
    pub tag: Vec<String>,
    /// Date/start: "2026-11-01", "tomorrow 9:00", "fri", "+3d"... A bare date makes an all-day task
    #[arg(
        short,
        long,
        alias = "due",
        alias = "start",
        allow_hyphen_values = true
    )]
    pub date: Option<String>,
    /// End of a span (inclusive day for all-day tasks)
    #[arg(long, allow_hyphen_values = true)]
    pub end: Option<String>,
    /// Length instead of --end, e.g. 30m, 1h30m
    #[arg(long)]
    pub duration: Option<String>,
    /// Force all-day on/off (default: all-day when --date has no time)
    #[arg(long)]
    pub all_day: Option<bool>,
    /// Reminder (repeatable): 0, -5m, -1h, -1d, 9:00 (all-day), -1d@9:00, or TRIGGER:...
    #[arg(short, long, allow_hyphen_values = true)]
    pub reminder: Vec<String>,
    /// Repeat: daily, weekly, monthly, yearly, weekdays, "every 2 weeks", "every mon,fri", or RRULE
    #[arg(long)]
    pub repeat: Option<String>,
    /// Repeat from the due date (default) or the completion date
    #[arg(long, value_enum)]
    pub repeat_from: Option<RepeatFrom>,
    /// Make this a subtask of another task
    #[arg(long)]
    pub parent: Option<String>,
    /// Checklist item (repeatable); makes the task a checklist
    #[arg(short, long)]
    pub item: Vec<String>,
    /// Create a note instead of a task
    #[arg(long)]
    pub note: bool,
    /// Kanban section (name or id)
    #[arg(long)]
    pub column: Option<String>,
    /// Pin the task
    #[arg(long)]
    pub pin: bool,
    /// Add at the top of the list
    #[arg(long, conflicts_with = "bottom")]
    pub top: bool,
    /// Add at the bottom of the list
    #[arg(long)]
    pub bottom: bool,
    /// Create already completed
    #[arg(long)]
    pub completed: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum RepeatFrom {
    Due,
    Completion,
}

impl RepeatFrom {
    fn code(self) -> &'static str {
        match self {
            RepeatFrom::Due => "0",
            RepeatFrom::Completion => "1",
        }
    }
}

pub fn add(ctx: &mut Ctx, a: &AddArgs) -> Result<()> {
    let sid = add_inner(ctx, a)?;
    let t = Task::load_by_sid(&ctx.conn, &sid)?.ok_or_else(|| anyhow!("task vanished (dry run?)"));
    match t {
        Ok(t) => {
            let v = model::task_json(&ctx.conn, &ctx.inbox_id, &t, true);
            out::done(
                ctx.json,
                v,
                &format!("Added task {} \"{}\" (sid {})", t.local_id, t.title, t.sid),
            );
        }
        Err(_) if ctx.dry_run => out::done(
            ctx.json,
            json!({"sid": sid, "dry_run": true}),
            &format!("Would add task (sid {sid})"),
        ),
        Err(e) => return Err(e),
    }
    Ok(())
}

/// Insert a task and return its sid.
pub fn add_inner(ctx: &mut Ctx, a: &AddArgs) -> Result<String> {
    let title = a.title.replace(['\n', '\r'], " ").trim().to_string();
    if title.is_empty() {
        bail!("Title is empty");
    }
    let start = a.date.as_deref().map(dates::parse_when).transpose()?;
    let end = a.end.as_deref().map(dates::parse_when).transpose()?;
    let dset = build_dates(start, end, a.all_day, a.duration.as_deref())?;
    let priority = a
        .priority
        .as_deref()
        .map(model::parse_priority)
        .transpose()?
        .unwrap_or(0);
    let repeat = a.repeat.as_deref().map(dates::parse_repeat).transpose()?;
    if repeat.is_some() && dset.start.is_none() {
        bail!("--repeat needs --date");
    }
    let reminders: Vec<String> = a
        .reminder
        .iter()
        .map(|r| dates::parse_reminder(r))
        .collect::<Result<_>>()?;
    if !reminders.is_empty() && dset.start.is_none() {
        bail!("--reminder needs --date");
    }
    let parent = a
        .parent
        .as_deref()
        .map(|p| model::resolve_task(&ctx.conn, &ctx.user_id, p))
        .transpose()?;
    let project = match (&a.project, &parent) {
        (Some(p), _) => model::resolve_project(&ctx.conn, &ctx.inbox_id, p)?,
        (None, Some(par)) => model::resolve_project(&ctx.conn, &ctx.inbox_id, &par.project_id)
            .unwrap_or_else(|_| model::Project::inbox(&ctx.inbox_id)),
        (None, None) => {
            let def: Option<String> = ctx
                .conn
                .query_row(
                    "SELECT projectId FROM TaskDefaultModel WHERE UserId = ? LIMIT 1",
                    [&ctx.user_id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten()
                .filter(|s: &String| !s.is_empty());
            match def {
                Some(d) => model::resolve_project(&ctx.conn, &ctx.inbox_id, &d)
                    .unwrap_or_else(|_| model::Project::inbox(&ctx.inbox_id)),
                None => model::Project::inbox(&ctx.inbox_id),
            }
        }
    };
    if project.closed {
        bail!("Project '{}' is archived", project.name);
    }
    if let Some(par) = &parent
        && par.project_id != project.sid
    {
        bail!(
            "A subtask must be in its parent's project ({})",
            model::project_name(&ctx.conn, &ctx.inbox_id, &par.project_id)
        );
    }
    let top = if a.top {
        true
    } else if a.bottom {
        false
    } else {
        default_add_to_top(&ctx.conn, &ctx.user_id)
    };
    let kind = if a.note {
        "NOTE"
    } else if !a.item.is_empty() {
        "CHECKLIST"
    } else {
        "TEXT"
    };

    let sid = ctx.write(|conn, env| {
        let tags = ensure_tags(conn, env, &a.tag)?;
        let column = match &a.column {
            Some(c) => Some(resolve_column(conn, &project.sid, c)?),
            None => match &parent {
                Some(p) => p.column_id.clone(),
                None => default_column(conn, &project.sid)?,
            },
        };
        let sid = ctx::new_object_id();
        let now = ctx::now_str();
        let sort = new_sort_order(conn, &project.sid, parent.as_ref().map(|p| p.sid.as_str()), top)?;
        let content = a.content.clone().unwrap_or_default();
        let (content_col, desc_col) = if kind == "CHECKLIST" { (String::new(), content) } else { (content, String::new()) };
        let status = if a.completed { 2 } else { 0 };
        conn.execute(
            "INSERT INTO TaskModel (id, projectId, parentId, priority, title, content, desc, isAllDay,
                repeatFlag, repeatFrom, progress, hasLocation, columnId, creator, userId, tag,
                completedTime, completedUserId, createdTime, startDate, modifiedTime, dueDate, deleted, status,
                kind, sortOrder, timeZone, attachmentCount, reminderCount, isFloating, isOpen, pinnedTime, imgMode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, 0, ?11, ?12, ?12, ?13, ?14, ?15, ?16, ?17, ?16, ?18,
                0, ?19, ?20, ?21, ?22, 0, 0, 0, 1, ?23, 0)",
            params![
                sid,
                project.sid,
                parent.as_ref().map(|p| p.sid.clone()),
                priority,
                title,
                content_col,
                desc_col,
                dset.all_day.map(|b| b as i64),
                repeat,
                repeat.as_ref().map(|_| a.repeat_from.unwrap_or(RepeatFrom::Due).code()),
                column,
                env.user_id,
                tags_json(&tags),
                if a.completed { Some(now.clone()) } else { None },
                if a.completed { Some(env.user_id.clone()) } else { None },
                now,
                dset.start,
                dset.due,
                status,
                kind,
                sort,
                env.time_zone,
                if a.pin { Some(pinned_value()) } else { None },
            ],
        )?;
        let task = Task::load_by_sid(conn, &sid)?.ok_or_else(|| anyhow!("insert failed"))?;
        if !reminders.is_empty() {
            set_reminders(conn, &task, &reminders)?;
        }
        for (i, item) in a.item.iter().enumerate() {
            conn.execute(
                "INSERT INTO TaskDetailItemModel (id, TaskId, TaskServerId, title, status, sortOrder)
                 VALUES (?, ?, ?, ?, 0, ?)",
                params![ctx::new_object_id(), task.local_id, sid, item, (i as i64) * SORT_STEP],
            )?;
        }
        ctx::add_sync_status(conn, env, &sid, sync::CREATE)?;
        if let Some(p) = &parent {
            add_child_id(conn, &p.sid, &sid, true)?;
            ctx::add_set_parent_status(conn, env, &sid, "")?;
        }
        Ok(sid)
    })?;
    Ok(sid)
}

// ---------------------------------------------------------------------------
// edit
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Default)]
pub struct EditArgs {
    /// Task id, sid, or exact title
    pub task: String,
    /// New title
    #[arg(long)]
    pub title: Option<String>,
    /// Replace the description
    #[arg(short, long)]
    pub content: Option<String>,
    /// Append a line to the description
    #[arg(long)]
    pub append: Option<String>,
    /// Priority: none, low, medium, high
    #[arg(long)]
    pub priority: Option<String>,
    /// Replace all tags (comma-separated; empty string clears)
    #[arg(long)]
    pub tags: Option<String>,
    /// Add a tag (repeatable)
    #[arg(long)]
    pub add_tag: Vec<String>,
    /// Remove a tag (repeatable)
    #[arg(long)]
    pub remove_tag: Vec<String>,
    /// New date/start (see `tasks add --help`)
    #[arg(
        short,
        long,
        alias = "due",
        alias = "start",
        allow_hyphen_values = true
    )]
    pub date: Option<String>,
    /// New end of the span (inclusive day for all-day tasks); keeps --date
    #[arg(long, allow_hyphen_values = true)]
    pub end: Option<String>,
    /// New length instead of --end, e.g. 30m, 1h30m
    #[arg(long)]
    pub duration: Option<String>,
    /// Make the task all-day (true) or timed (false)
    #[arg(long)]
    pub all_day: Option<bool>,
    /// Remove date, end, reminders and repeat
    #[arg(long)]
    pub clear_date: bool,
    /// Replace reminders (repeatable)
    #[arg(short, long, allow_hyphen_values = true)]
    pub reminder: Vec<String>,
    /// Remove all reminders
    #[arg(long)]
    pub clear_reminders: bool,
    /// Repeat: daily, weekly, monthly, yearly, weekdays, "every 2 weeks", "every mon,fri", or RRULE
    #[arg(long)]
    pub repeat: Option<String>,
    /// Repeat from the due date or the completion date
    #[arg(long, value_enum)]
    pub repeat_from: Option<RepeatFrom>,
    /// Stop repeating
    #[arg(long)]
    pub clear_repeat: bool,
    /// Move to another project
    #[arg(short, long)]
    pub project: Option<String>,
    /// Make a subtask of this task
    #[arg(long)]
    pub parent: Option<String>,
    /// Detach from its parent
    #[arg(long)]
    pub no_parent: bool,
    /// Kanban section
    #[arg(long)]
    pub column: Option<String>,
    /// Pin the task
    #[arg(long, conflicts_with = "unpin")]
    pub pin: bool,
    /// Unpin the task
    #[arg(long)]
    pub unpin: bool,
    /// Change kind: task, note, checklist
    #[arg(long)]
    pub kind: Option<String>,
    /// Move to top of its list
    #[arg(long, conflicts_with = "bottom")]
    pub top: bool,
    /// Move to bottom of its list
    #[arg(long)]
    pub bottom: bool,
}

pub fn edit(ctx: &mut Ctx, a: &EditArgs) -> Result<()> {
    let t = model::resolve_task(&ctx.conn, &ctx.user_id, &a.task)?;
    let priority = a
        .priority
        .as_deref()
        .map(model::parse_priority)
        .transpose()?;
    let new_project = a
        .project
        .as_deref()
        .map(|p| model::resolve_project(&ctx.conn, &ctx.inbox_id, p))
        .transpose()?;
    let new_parent = a
        .parent
        .as_deref()
        .map(|p| model::resolve_task(&ctx.conn, &ctx.user_id, p))
        .transpose()?;
    let start = a.date.as_deref().map(dates::parse_when).transpose()?;
    let end = a.end.as_deref().map(dates::parse_when).transpose()?;
    let repeat = a.repeat.as_deref().map(dates::parse_repeat).transpose()?;
    let reminders: Vec<String> = a
        .reminder
        .iter()
        .map(|r| dates::parse_reminder(r))
        .collect::<Result<_>>()?;
    if let Some(p) = &new_parent
        && (p.sid == t.sid || descendants(&ctx.conn, &t.sid)?.contains(&p.sid))
    {
        bail!("A task cannot be its own ancestor");
    }
    if let Some(p) = &new_project
        && p.closed
    {
        bail!("Project '{}' is archived", p.name);
    }

    ctx.write(|conn, env| {
        let sid = t.sid.as_str();
        let mut changed = false;
        if let Some(title) = &a.title {
            conn.execute("UPDATE TaskModel SET title = ? WHERE id = ?", params![title.replace(['\n', '\r'], " ").trim(), sid])?;
            changed = true;
        }
        let body_col = if t.kind == "CHECKLIST" { "desc" } else { "content" };
        if let Some(c) = &a.content {
            conn.execute(&format!("UPDATE TaskModel SET {body_col} = ? WHERE id = ?"), params![c, sid])?;
            changed = true;
        }
        if let Some(app) = &a.append {
            let cur = if t.kind == "CHECKLIST" { &t.desc } else { &t.content };
            let new = if cur.trim().is_empty() { app.clone() } else { format!("{}\n{}", cur.trim_end(), app) };
            conn.execute(&format!("UPDATE TaskModel SET {body_col} = ? WHERE id = ?"), params![new, sid])?;
            changed = true;
        }
        if let Some(p) = priority {
            conn.execute("UPDATE TaskModel SET priority = ? WHERE id = ?", params![p, sid])?;
            changed = true;
        }
        if a.tags.is_some() || !a.add_tag.is_empty() || !a.remove_tag.is_empty() {
            let mut tags: Vec<String> = match &a.tags {
                Some(s) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
                None => t.tags.clone(),
            };
            tags.extend(a.add_tag.iter().flat_map(|x| x.split(',')).map(|x| x.trim().to_string()));
            let remove: Vec<String> = a.remove_tag.iter().map(|x| x.trim_start_matches('#').to_lowercase()).collect();
            tags.retain(|x| !remove.contains(&x.to_lowercase()));
            let tags = ensure_tags(conn, env, &tags)?;
            conn.execute("UPDATE TaskModel SET tag = ? WHERE id = ?", params![tags_json(&tags), sid])?;
            changed = true;
        }
        if a.clear_date {
            conn.execute(
                "UPDATE TaskModel SET startDate = NULL, dueDate = NULL, isAllDay = NULL, repeatFlag = NULL,
                    repeatFrom = NULL, exDates = NULL, remindTime = NULL WHERE id = ?",
                [sid],
            )?;
            set_reminders(conn, &t, &[])?;
            changed = true;
        }
        if start.is_some() || end.is_some() || a.duration.is_some() || a.all_day.is_some() {
            // Fill unspecified parts from the existing task.
            let cur_start = t.start.as_deref().and_then(dates::parse_db_dt).map(|dt| When { dt, has_time: !t.all_day });
            let s = start.or(cur_start).ok_or_else(|| anyhow!("Task has no date; pass --date"))?;
            let e = match (&end, &a.duration) {
                (Some(e), _) => Some(*e),
                (None, Some(_)) => None,
                (None, None) if start.is_none() => t.due.as_deref().and_then(dates::parse_db_dt).map(|dt| {
                    // keep the span; convert the stored exclusive all-day end back to inclusive
                    let dt = if t.all_day { dt - Duration::days(1) } else { dt };
                    When { dt, has_time: !t.all_day }
                }),
                // moving the start keeps the same length
                (None, None) => match (t.start.as_deref().and_then(dates::parse_db_dt), t.due.as_deref().and_then(dates::parse_db_dt)) {
                    (Some(os), Some(oe)) => {
                        let len = oe - os;
                        let base = if s.has_time || t.all_day { s.dt } else { s.dt.date().and_time(os.time()) };
                        let ne = base + len;
                        Some(When { dt: if t.all_day { ne - Duration::days(1) } else { ne }, has_time: !t.all_day })
                    }
                    _ => None,
                },
            };
            let all_day = a.all_day.or(if start.is_some() { None } else { Some(t.all_day) });
            let d = build_dates(Some(s), e, all_day, a.duration.as_deref())?;
            conn.execute(
                "UPDATE TaskModel SET startDate = ?, dueDate = ?, isAllDay = ?, remindTime = NULL,
                    timeZone = COALESCE(NULLIF(timeZone, ''), ?) WHERE id = ?",
                params![d.start, d.due, d.all_day.map(|b| b as i64), env.time_zone, sid],
            )?;
            changed = true;
        }
        if a.clear_reminders {
            set_reminders(conn, &t, &[])?;
            changed = true;
        } else if !reminders.is_empty() {
            set_reminders(conn, &t, &reminders)?;
            changed = true;
        }
        if a.clear_repeat {
            conn.execute("UPDATE TaskModel SET repeatFlag = NULL, repeatFrom = NULL, exDates = NULL WHERE id = ?", [sid])?;
            changed = true;
        } else if repeat.is_some() || a.repeat_from.is_some() {
            let has_date: bool = conn.query_row(
                "SELECT startDate IS NOT NULL AND startDate <> '' FROM TaskModel WHERE id = ?",
                [sid],
                |r| r.get(0),
            )?;
            if !has_date {
                bail!("--repeat needs the task to have a date");
            }
            let rule = repeat.clone().or(t.repeat.clone()).ok_or_else(|| anyhow!("Task does not repeat"))?;
            let from = a.repeat_from.map(|r| r.code().to_string()).or(t.repeat_from.clone()).unwrap_or_else(|| "0".into());
            conn.execute("UPDATE TaskModel SET repeatFlag = ?, repeatFrom = ? WHERE id = ?", params![rule, from, sid])?;
            changed = true;
        }
        if let Some(k) = &a.kind {
            let k = match k.to_ascii_lowercase().as_str() {
                "task" | "text" => "TEXT",
                "note" => "NOTE",
                "checklist" => "CHECKLIST",
                other => bail!("unknown kind '{other}'"),
            };
            set_kind(conn, &t, k)?;
            changed = true;
        }
        if a.pin {
            conn.execute("UPDATE TaskModel SET pinnedTime = ? WHERE id = ?", params![pinned_value(), sid])?;
            changed = true;
        }
        if a.unpin {
            conn.execute("UPDATE TaskModel SET pinnedTime = NULL WHERE id = ?", [sid])?;
            changed = true;
        }
        if changed {
            touch(conn, env, sid)?;
        }
        // Structural changes carry their own sync markers.
        if a.no_parent {
            set_parent(conn, env, &t, None)?;
        }
        if let Some(p) = &new_parent {
            if p.project_id != t.project_id && new_project.is_none() {
                move_task(conn, env, &t, &p.project_id, true)?;
            }
            let t2 = Task::load_by_sid(conn, sid)?.unwrap();
            set_parent(conn, env, &t2, Some(p))?;
        }
        if let Some(p) = &new_project
            && p.sid != t.project_id {
                let t2 = Task::load_by_sid(conn, sid)?.unwrap();
                if new_parent.is_none() && t2.parent_id.as_deref().is_some_and(|x| !x.is_empty()) {
                    set_parent(conn, env, &t2, None)?;
                }
                let t3 = Task::load_by_sid(conn, sid)?.unwrap();
                move_task(conn, env, &t3, &p.sid, a.top || !a.bottom)?;
            }
        if let Some(c) = &a.column {
            let proj: String = conn.query_row("SELECT projectId FROM TaskModel WHERE id = ?", [sid], |r| r.get(0))?;
            let col = resolve_column(conn, &proj, c)?;
            conn.execute("UPDATE TaskModel SET columnId = ? WHERE id = ?", params![col, sid])?;
            touch(conn, env, sid)?;
        }
        if (a.top || a.bottom) && new_project.is_none() {
            let cur = Task::load_by_sid(conn, sid)?.unwrap();
            let order = new_sort_order(conn, &cur.project_id, cur.parent_id.as_deref().filter(|p| !p.is_empty()), a.top)?;
            conn.execute("UPDATE TaskModel SET sortOrder = ? WHERE id = ?", params![order, sid])?;
            touch(conn, env, sid)?;
        }
        Ok(())
    })?;
    let t = Task::load_by_sid(&ctx.conn, &t.sid)?.unwrap();
    out::done(
        ctx.json,
        model::task_json(&ctx.conn, &ctx.inbox_id, &t, true),
        &format!("Updated task {} \"{}\"", t.local_id, t.title),
    );
    Ok(())
}

fn set_kind(conn: &Connection, t: &Task, kind: &str) -> Result<()> {
    if t.kind == kind || (kind == "TEXT" && t.kind.is_empty()) {
        return Ok(());
    }
    if kind == "CHECKLIST" {
        let desc = if t.desc.is_empty() {
            t.content.clone()
        } else {
            t.desc.clone()
        };
        conn.execute(
            "UPDATE TaskModel SET kind = 'CHECKLIST', desc = ? WHERE id = ?",
            params![desc, t.sid],
        )?;
        return Ok(());
    }
    if t.kind == "CHECKLIST" {
        // Fold the checklist back into text, like the app does.
        let items = model::task_items(conn, &t.sid)?;
        let mut body = t.desc.clone();
        for i in &items {
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(&format!(
                "- [{}] {}",
                if i.status == 1 { "x" } else { " " },
                i.title
            ));
        }
        conn.execute(
            "DELETE FROM TaskDetailItemModel WHERE TaskServerId = ?",
            [&t.sid],
        )?;
        conn.execute(
            "UPDATE TaskModel SET kind = ?, content = ?, desc = '', progress = 0 WHERE id = ?",
            params![kind, body, t.sid],
        )?;
        return Ok(());
    }
    conn.execute(
        "UPDATE TaskModel SET kind = ? WHERE id = ?",
        params![kind, t.sid],
    )?;
    Ok(())
}

fn set_parent(conn: &Connection, env: &WriteEnv, t: &Task, parent: Option<&Task>) -> Result<()> {
    let old = t.parent_id.clone().filter(|p| !p.is_empty());
    let new = parent.map(|p| p.sid.clone());
    if old == new {
        return Ok(());
    }
    let column = parent
        .and_then(|p| p.column_id.clone())
        .or(t.column_id.clone());
    conn.execute(
        "UPDATE TaskModel SET parentId = ?, columnId = ?, modifiedTime = ? WHERE id = ?",
        params![new, column, ctx::now_str(), t.sid],
    )?;
    ctx::add_set_parent_status(conn, env, &t.sid, old.as_deref().unwrap_or(""))?;
    if let Some(n) = &new {
        add_child_id(conn, n, &t.sid, true)?;
    }
    if let Some(o) = &old {
        add_child_id(conn, o, &t.sid, false)?;
    }
    Ok(())
}

pub fn move_many(ctx: &mut Ctx, keys: &[String], project: &str) -> Result<()> {
    let p = model::resolve_project(&ctx.conn, &ctx.inbox_id, project)?;
    if p.closed {
        bail!("Project '{}' is archived", p.name);
    }
    let tasks: Vec<Task> = keys
        .iter()
        .map(|k| model::resolve_task(&ctx.conn, &ctx.user_id, k))
        .collect::<Result<_>>()?;
    ctx.write(|conn, env| {
        for t in &tasks {
            let t = Task::load_by_sid(conn, &t.sid)?.unwrap();
            if t.project_id == p.sid {
                continue;
            }
            // A subtask moved on its own leaves its parent.
            let parent_moving = t
                .parent_id
                .as_deref()
                .is_some_and(|par| tasks.iter().any(|o| o.sid == par));
            if t.parent_id.as_deref().is_some_and(|x| !x.is_empty()) && !parent_moving {
                set_parent(conn, env, &t, None)?;
            }
            let t = Task::load_by_sid(conn, &t.sid)?.unwrap();
            if t.project_id != p.sid {
                move_task(conn, env, &t, &p.sid, true)?;
            }
        }
        Ok(())
    })?;
    out::done(
        ctx.json,
        json!({"action": "move", "project_sid": p.sid, "project": p.name, "sids": tasks.iter().map(|t| &t.sid).collect::<Vec<_>>()}),
        &format!("Moved {} task(s) to \"{}\"", tasks.len(), p.name),
    );
    Ok(())
}

/// Move a task (and its subtasks) to another project.
fn move_task(conn: &Connection, env: &WriteEnv, t: &Task, project: &str, top: bool) -> Result<()> {
    let mut all = vec![t.sid.clone()];
    all.extend(descendants(conn, &t.sid)?);
    let column = default_column(conn, project)?;
    let order = new_sort_order(conn, project, None, top)?;
    for (i, sid) in all.iter().enumerate() {
        let from: String =
            conn.query_row("SELECT projectId FROM TaskModel WHERE id = ?", [sid], |r| {
                r.get(0)
            })?;
        if i == 0 {
            conn.execute(
                "UPDATE TaskModel SET projectId = ?, columnId = ?, sortOrder = ?, modifiedTime = ? WHERE id = ?",
                params![project, column, order, ctx::now_str(), sid],
            )?;
        } else {
            conn.execute(
                "UPDATE TaskModel SET projectId = ?, columnId = ?, modifiedTime = ? WHERE id = ?",
                params![project, column, ctx::now_str(), sid],
            )?;
        }
        ctx::add_move_status(conn, env, sid, &from, sync::MOVE)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// status changes
// ---------------------------------------------------------------------------

pub struct CloseOpts {
    /// For repeating tasks: close the whole series instead of one occurrence.
    pub whole_series: bool,
    pub at: Option<NaiveDateTime>,
}

pub fn set_status(ctx: &mut Ctx, keys: &[String], status: i64, opts: CloseOpts) -> Result<()> {
    let tasks: Vec<Task> = keys
        .iter()
        .map(|k| model::resolve_task(&ctx.conn, &ctx.user_id, k))
        .collect::<Result<_>>()?;
    let results = ctx.write(|conn, env| {
        let mut results = Vec::new();
        for t in &tasks {
            let r = close_one(conn, env, t, status, &opts)?;
            results.push(r);
        }
        Ok(results)
    })?;
    let mut out_rows = Vec::new();
    for (sid, clone) in &results {
        let t = Task::load_by_sid(&ctx.conn, sid)?.unwrap();
        let mut v = model::task_json(&ctx.conn, &ctx.inbox_id, &t, false);
        if let Some(c) = clone {
            let ct = Task::load_by_sid(&ctx.conn, c)?.unwrap();
            v["closed_occurrence"] = model::task_json(&ctx.conn, &ctx.inbox_id, &ct, false);
        }
        if !ctx.json {
            match clone {
                Some(_) => println!(
                    "Closed one occurrence of {} \"{}\"; next: {}",
                    t.local_id,
                    t.title,
                    t.start.as_deref().unwrap_or("-")
                ),
                None => println!(
                    "Task {} \"{}\" is now {}",
                    t.local_id,
                    t.title,
                    model::status_name(t.status)
                ),
            }
        }
        out_rows.push(v);
    }
    if ctx.json {
        out::json(&json!(out_rows));
    }
    Ok(())
}

/// Returns (task sid, sid of the completed occurrence for repeating tasks).
fn close_one(
    conn: &Connection,
    env: &WriteEnv,
    t: &Task,
    status: i64,
    opts: &CloseOpts,
) -> Result<(String, Option<String>)> {
    let now = opts
        .at
        .map(|d| ctx::fmt_dt(&d))
        .unwrap_or_else(ctx::now_str);
    if t.status == status && !(status != 0 && t.is_repeating() && !opts.whole_series) {
        return Ok((t.sid.clone(), None));
    }
    if status == 0 {
        conn.execute(
            "UPDATE TaskModel SET status = 0, completedTime = NULL, completedUserId = NULL, modifiedTime = ? WHERE id = ?",
            params![now, t.sid],
        )?;
        ctx::add_sync_status(conn, env, &t.sid, sync::MODIFY)?;
        // Reopening a subtask reopens closed ancestors (TryUnCompleteParentTask).
        let mut parent = t.parent_id.clone().filter(|p| !p.is_empty());
        while let Some(p) = parent {
            let pt = Task::load_by_sid(conn, &p)?;
            match pt {
                Some(pt) if pt.status != 0 => {
                    conn.execute(
                        "UPDATE TaskModel SET status = 0, completedTime = NULL, completedUserId = NULL, modifiedTime = ? WHERE id = ?",
                        params![now, pt.sid],
                    )?;
                    ctx::add_sync_status(conn, env, &pt.sid, sync::MODIFY)?;
                    parent = pt.parent_id.filter(|p| !p.is_empty());
                }
                _ => break,
            }
        }
        return Ok((t.sid.clone(), None));
    }

    if t.is_repeating()
        && !opts.whole_series
        && let Some(clone) = close_occurrence(conn, env, t, status, &now)?
    {
        return Ok((t.sid.clone(), Some(clone)));
    }
    // No further occurrences: fall through and close the task itself.
    conn.execute(
        "UPDATE TaskModel SET status = ?, completedTime = ?, completedUserId = ?, remindTime = NULL, modifiedTime = ? WHERE id = ?",
        params![status, now, env.user_id, now, t.sid],
    )?;
    ctx::add_sync_status(conn, env, &t.sid, sync::MODIFY)?;
    // Closing a parent closes its open subtasks (TryCloseSubTask).
    for kid in descendants(conn, &t.sid)? {
        let n = conn.execute(
            "UPDATE TaskModel SET status = ?, completedTime = ?, completedUserId = ?, modifiedTime = ? WHERE id = ? AND status = 0",
            params![status, now, env.user_id, now, kid],
        )?;
        if n > 0 {
            ctx::add_sync_status(conn, env, &kid, sync::MODIFY)?;
        }
    }
    Ok((t.sid.clone(), None))
}

/// Complete one occurrence of a repeating task: insert a closed copy and move the
/// original to its next date. Returns None when the series has no next date.
fn close_occurrence(
    conn: &Connection,
    env: &WriteEnv,
    t: &Task,
    status: i64,
    now: &str,
) -> Result<Option<String>> {
    let rule_text = t.repeat.clone().unwrap_or_default();
    let rule = Rrule::parse(&rule_text)?;
    rule.check_supported()
        .map_err(|e| anyhow!("{e}; use --whole-series to close the task outright"))?;
    if rule.count == Some(1) || rule.count == Some(0) {
        return Ok(None);
    }
    let Some(start) = t.start.as_deref().and_then(dates::parse_db_dt) else {
        return Ok(None);
    };
    let from_completion = t.repeat_from.as_deref() == Some("1");
    let after = if from_completion {
        dates::today()
    } else {
        start.date()
    };
    let anchor = if from_completion {
        dates::today()
    } else {
        start.date()
    };
    let next = if from_completion {
        // first occurrence strictly after completion day, counted from it
        rule.next_after(anchor, anchor)
    } else {
        rule.next_after(anchor, after)
    };
    let Some(next_day) = next else {
        return Ok(None);
    };
    let new_start = next_day.and_time(start.time());
    let new_due = t
        .due
        .as_deref()
        .and_then(dates::parse_db_dt)
        .map(|d| new_start + (d - start));

    // The closed copy keeps the occurrence's dates.
    let clone_sid = ctx::new_object_id();
    let order = {
        let mut stmt = conn.prepare(
            "SELECT sortOrder FROM TaskModel WHERE projectId = ? AND sortOrder > ? AND (deleted = 0 OR deleted IS NULL) ORDER BY sortOrder LIMIT 1",
        )?;
        let nxt: Option<i64> = stmt
            .query_row(params![t.project_id, t.sort_order], |r| r.get(0))
            .optional()?;
        match nxt {
            Some(n) => t.sort_order + (n - t.sort_order) / 2,
            None => t.sort_order + SORT_STEP,
        }
    };
    let cols: Vec<&str> = COPY_COLS.split(',').map(str::trim).collect();
    let select: Vec<String> =
        cols.iter()
            .map(|c| match *c {
                "id" => ":sid".to_string(),
                "status" => ":status".to_string(),
                "createdTime" | "completedTime" | "modifiedTime" => ":now".to_string(),
                "sortOrder" => ":ord".to_string(),
                "repeatFlag" | "repeatFrom" | "exDates" | "etag" | "childrenString"
                | "pinnedTime" => "NULL".to_string(),
                other => format!("\"{other}\""),
            })
            .collect();
    conn.execute(
        &format!(
            "INSERT INTO TaskModel ({}, repeatTaskId, completedUserId, creator)
             SELECT {}, :orig, :user, :user FROM TaskModel WHERE id = :orig",
            cols.iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(", "),
            select.join(", ")
        ),
        rusqlite::named_params! {
            ":sid": clone_sid, ":status": status, ":now": now, ":ord": order,
            ":orig": t.sid, ":user": env.user_id,
        },
    )?;
    let clone_local: i64 = conn.query_row(
        "SELECT _Id FROM TaskModel WHERE id = ?",
        [&clone_sid],
        |r| r.get(0),
    )?;
    // Copy checklist items to the closed occurrence, then reset the original's.
    for it in model::task_items(conn, &t.sid)? {
        conn.execute(
            "INSERT INTO TaskDetailItemModel (id, TaskId, TaskServerId, title, status, sortOrder, isAllDay, completedTime, startDate)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![ctx::new_object_id(), clone_local, clone_sid, it.title, it.status, it.sort_order, it.all_day as i64, it.completed, it.start],
        )?;
    }
    conn.execute(
        "UPDATE TaskDetailItemModel SET status = 0, completedTime = NULL WHERE TaskServerId = ?",
        [&t.sid],
    )?;
    ctx::add_sync_status(conn, env, &clone_sid, sync::CREATE)?;

    conn.execute(
        "UPDATE TaskModel SET startDate = ?, dueDate = ?, repeatFlag = ?, progress = 0, remindTime = NULL, modifiedTime = ? WHERE id = ?",
        params![
            ctx::fmt_dt(&new_start),
            new_due.map(|d| ctx::fmt_dt(&d)),
            Rrule::after_one_occurrence(&rule_text),
            now,
            t.sid
        ],
    )?;
    ctx::add_sync_status(conn, env, &t.sid, sync::MODIFY)?;
    Ok(Some(clone_sid))
}

/// Columns copied when cloning a task row.
const COPY_COLS: &str = "id, projectId, imgMode, parentId, priority, title, content, isAllDay, repeatFlag, \
    reminder, progress, etag, hasLocation, columnId, exDates, repeatFrom, \
    assignee, userId, tag, completedTime, createdTime, startDate, modifiedTime, dueDate, deleted, status, \
    kind, sortOrder, desc, timeZone, attachmentCount, reminderCount, isFloating, childrenString, isOpen, pinnedTime";

// ---------------------------------------------------------------------------
// trash / restore / purge
// ---------------------------------------------------------------------------

pub fn trash(ctx: &mut Ctx, keys: &[String], forever: bool) -> Result<()> {
    let tasks: Vec<Task> = keys
        .iter()
        .map(|k| model::resolve_task(&ctx.conn, &ctx.user_id, k))
        .collect::<Result<_>>()?;
    let affected = ctx.write(|conn, env| {
        let mut affected = Vec::new();
        for t in &tasks {
            let mut ids = vec![t.sid.clone()];
            ids.extend(descendants(conn, &t.sid)?);
            for sid in ids {
                trash_one(conn, env, &sid, forever)?;
                affected.push(sid);
            }
        }
        Ok(affected)
    })?;
    let verb = if forever {
        "Deleted forever"
    } else {
        "Moved to trash"
    };
    out::done(
        ctx.json,
        json!({"action": if forever {"delete_forever"} else {"trash"}, "sids": affected}),
        &format!("{verb}: {} task(s)", affected.len()),
    );
    Ok(())
}

fn trash_one(conn: &Connection, env: &WriteEnv, sid: &str, forever: bool) -> Result<()> {
    let now = ctx::now_str();
    if forever {
        conn.execute(
            "UPDATE TaskModel SET deleted = 2, attendId = NULL, modifiedTime = ? WHERE id = ?",
            params![now, sid],
        )?;
        let never_synced: bool = conn
            .query_row(
                "SELECT 1 FROM SyncStatusModel WHERE EntityId = ? AND Type = 4 AND UserId = ?",
                params![sid, env.user_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        conn.execute(
            "DELETE FROM SyncStatusModel WHERE EntityId = ? AND UserId = ?",
            params![sid, env.user_id],
        )?;
        if !never_synced {
            conn.execute(
                "INSERT INTO SyncStatusModel (UserId, EntityId, Type, ModifyPoint) VALUES (?, ?, 6, ?)",
                params![env.user_id, sid, ctx::now_ticks()],
            )?;
        }
    } else {
        conn.execute(
            "UPDATE TaskModel SET deleted = 1, attendId = NULL, modifiedTime = ? WHERE id = ?",
            params![now, sid],
        )?;
        ctx::add_sync_status(conn, env, sid, sync::TRASH)?;
    }
    Ok(())
}

pub fn restore(ctx: &mut Ctx, keys: &[String]) -> Result<()> {
    let tasks: Vec<Task> = keys
        .iter()
        .map(|k| model::resolve_task(&ctx.conn, &ctx.user_id, k))
        .collect::<Result<_>>()?;
    let inbox = ctx.inbox_id.clone();
    ctx.write(|conn, env| {
        for t in &tasks {
            let pending_trash: bool = conn
                .query_row(
                    "SELECT 1 FROM SyncStatusModel WHERE EntityId = ? AND Type IN (4, 5, 6) AND UserId = ?",
                    params![t.sid, env.user_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            ctx::remove_sync_status(conn, env, &t.sid, sync::TRASH)?;
            ctx::remove_sync_status(conn, env, &t.sid, sync::DELETE_FOREVER)?;
            // If its project is gone, restore into the inbox (BatchRestoreProject).
            let project_ok: bool = t.project_id == inbox
                || conn
                    .query_row(
                        "SELECT 1 FROM ProjectModel WHERE id = ? AND (delete_status IS NULL OR delete_status = 0)",
                        [&t.project_id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
            let project = if project_ok { t.project_id.clone() } else { inbox.clone() };
            conn.execute(
                "UPDATE TaskModel SET deleted = 0, projectId = ?, modifiedTime = ? WHERE id = ?",
                params![project, ctx::now_str(), t.sid],
            )?;
            if !pending_trash {
                ctx::add_move_status(conn, env, &t.sid, &t.project_id, sync::RESTORE)?;
            }
        }
        Ok(())
    })?;
    out::done(
        ctx.json,
        json!({"action": "restore", "sids": tasks.iter().map(|t| &t.sid).collect::<Vec<_>>()}),
        &format!("Restored {} task(s)", tasks.len()),
    );
    Ok(())
}

pub fn trash_list(ctx: &Ctx) -> Result<()> {
    let mut stmt = ctx.conn.prepare(&format!(
        "SELECT {TASK_COLS} FROM TaskModel WHERE deleted = 1 AND (userId = ? OR userId IS NULL) ORDER BY modifiedTime DESC"
    ))?;
    let tasks: Vec<Task> = stmt
        .query_map([&ctx.user_id], Task::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    if ctx.json {
        out::json(&json!(
            tasks
                .iter()
                .map(|t| model::task_json(&ctx.conn, &ctx.inbox_id, t, false))
                .collect::<Vec<_>>()
        ));
        return Ok(());
    }
    let rows = tasks
        .iter()
        .map(|t| {
            vec![
                t.local_id.to_string(),
                t.title.clone(),
                model::project_name(&ctx.conn, &ctx.inbox_id, &t.project_id),
                t.modified.clone().unwrap_or_default(),
            ]
        })
        .collect();
    out::table(
        &["ID", "Title", "Project", "Trashed"],
        rows,
        "Trash is empty.",
    );
    Ok(())
}

pub fn trash_empty(ctx: &mut Ctx) -> Result<()> {
    let sids: Vec<String> = {
        let mut stmt = ctx.conn.prepare(
            "SELECT id FROM TaskModel WHERE deleted = 1 AND (userId = ? OR userId IS NULL)",
        )?;
        stmt.query_map([&ctx.user_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    ctx.write(|conn, env| {
        for s in &sids {
            trash_one(conn, env, s, true)?;
        }
        Ok(())
    })?;
    out::done(
        ctx.json,
        json!({"action": "empty_trash", "sids": sids}),
        &format!("Emptied trash ({} task(s))", sids.len()),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// duplicate
// ---------------------------------------------------------------------------

pub fn duplicate(ctx: &mut Ctx, key: &str) -> Result<()> {
    let t = model::resolve_task(&ctx.conn, &ctx.user_id, key)?;
    let sid = ctx.write(|conn, env| {
        let sid = ctx::new_object_id();
        let order = t.sort_order + 1;
        let now = ctx::now_str();
        conn.execute(
            "INSERT INTO TaskModel (id, projectId, parentId, priority, title, content, desc, isAllDay, repeatFlag,
                repeatFrom, progress, hasLocation, columnId, creator, userId, tag, createdTime, startDate,
                modifiedTime, dueDate, deleted, status, kind, sortOrder, timeZone, attachmentCount, reminderCount,
                isFloating, isOpen, imgMode)
             SELECT ?1, projectId, parentId, priority, title, content, desc, isAllDay, repeatFlag, repeatFrom, 0, 0,
                columnId, ?2, ?2, tag, ?3, startDate, ?3, dueDate, 0, 0, kind, ?4, timeZone, 0, reminderCount,
                isFloating, 1, 0 FROM TaskModel WHERE id = ?5",
            params![sid, env.user_id, now, order, t.sid],
        )?;
        let local: i64 = conn.query_row("SELECT _Id FROM TaskModel WHERE id = ?", [&sid], |r| r.get(0))?;
        for it in model::task_items(conn, &t.sid)? {
            conn.execute(
                "INSERT INTO TaskDetailItemModel (id, TaskId, TaskServerId, title, status, sortOrder, isAllDay, startDate)
                 VALUES (?, ?, ?, ?, 0, ?, ?, ?)",
                params![ctx::new_object_id(), local, sid, it.title, it.sort_order, it.all_day as i64, it.start],
            )?;
        }
        for trig in model::task_reminders(conn, &t.sid)? {
            conn.execute(
                "INSERT INTO TaskReminderModel (id, Taskid, taskserverid, trigger) VALUES (?, ?, ?, ?)",
                params![ctx::new_object_id(), local, sid, trig],
            )?;
        }
        ctx::add_sync_status(conn, env, &sid, sync::CREATE)?;
        if let Some(p) = t.parent_id.as_deref().filter(|p| !p.is_empty()) {
            add_child_id(conn, p, &sid, true)?;
            ctx::add_set_parent_status(conn, env, &sid, "")?;
        }
        Ok(sid)
    })?;
    let nt = Task::load_by_sid(&ctx.conn, &sid)?.unwrap();
    out::done(
        ctx.json,
        model::task_json(&ctx.conn, &ctx.inbox_id, &nt, true),
        &format!("Duplicated as task {} (sid {})", nt.local_id, nt.sid),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Checklist helpers used by `items`
// ---------------------------------------------------------------------------

pub fn recompute_progress(conn: &Connection, env: &WriteEnv, task: &Task) -> Result<()> {
    let items = model::task_items(conn, &task.sid)?;
    if items.is_empty() {
        // Back to a plain task, description moves back to content.
        if task.kind == "CHECKLIST" {
            conn.execute(
                "UPDATE TaskModel SET kind = 'TEXT', progress = 0, content = desc, desc = '' WHERE id = ?",
                [&task.sid],
            )?;
        }
    } else {
        let done = items.iter().filter(|i| i.status == 1).count();
        let progress = (done * 100 / items.len()) as i64;
        if task.kind != "CHECKLIST" {
            let desc = if task.desc.is_empty() {
                task.content.clone()
            } else {
                task.desc.clone()
            };
            conn.execute(
                "UPDATE TaskModel SET kind = 'CHECKLIST', desc = ? WHERE id = ?",
                params![desc, task.sid],
            )?;
        }
        conn.execute(
            "UPDATE TaskModel SET progress = ? WHERE id = ?",
            params![progress, task.sid],
        )?;
    }
    touch(conn, env, &task.sid)
}

pub fn complete_for_items(conn: &Connection, env: &WriteEnv, task: &Task) -> Result<()> {
    close_one(
        conn,
        env,
        task,
        2,
        &CloseOpts {
            whole_series: false,
            at: None,
        },
    )
    .map(|_| ())
}

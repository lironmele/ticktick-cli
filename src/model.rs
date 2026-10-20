//! Row types loaded from the TickTick database, id resolution, and JSON views.

use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Value, json};

pub const TASK_COLS: &str = "_Id, id, projectId, title, content, desc, kind, status, priority, \
    startDate, dueDate, isAllDay, repeatFlag, repeatFrom, tag, parentId, childrenString, sortOrder, \
    deleted, createdTime, modifiedTime, completedTime, columnId, pinnedTime, progress, timeZone, \
    reminderCount, isFloating, userId, etag, exDates, assignee";

#[derive(Debug, Clone)]
#[allow(dead_code)] // full row, some fields only for debugging
pub struct Task {
    pub local_id: i64,
    pub sid: String,
    pub project_id: String,
    pub title: String,
    pub content: String,
    pub desc: String,
    pub kind: String,
    pub status: i64,
    pub priority: i64,
    pub start: Option<String>,
    pub due: Option<String>,
    pub all_day: bool,
    pub repeat: Option<String>,
    pub repeat_from: Option<String>,
    pub tags: Vec<String>,
    pub parent_id: Option<String>,
    pub children: Vec<String>,
    pub sort_order: i64,
    pub deleted: i64,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub completed: Option<String>,
    pub column_id: Option<String>,
    pub pinned: Option<String>,
    pub progress: i64,
    pub time_zone: Option<String>,
    pub reminder_count: i64,
    pub floating: bool,
    pub user_id: Option<String>,
    pub etag: Option<String>,
    pub ex_dates: Option<String>,
    pub assignee: Option<String>,
}

fn opt_str(r: &Row, i: usize) -> rusqlite::Result<Option<String>> {
    // Dates may come back as text or (rarely) other types; normalise to text.
    use rusqlite::types::ValueRef;
    Ok(match r.get_ref(i)? {
        ValueRef::Null => None,
        ValueRef::Text(t) => {
            let s = String::from_utf8_lossy(t).to_string();
            if s.is_empty() { None } else { Some(s) }
        }
        ValueRef::Integer(n) => Some(n.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Blob(_) => None,
    })
}

fn int(r: &Row, i: usize) -> rusqlite::Result<i64> {
    use rusqlite::types::ValueRef;
    Ok(match r.get_ref(i)? {
        ValueRef::Integer(n) => n,
        ValueRef::Real(f) => f as i64,
        ValueRef::Text(t) => String::from_utf8_lossy(t).parse().unwrap_or(0),
        _ => 0,
    })
}

pub fn parse_json_list(s: Option<&str>) -> Vec<String> {
    s.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
}

impl Task {
    pub fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            local_id: r.get(0)?,
            sid: opt_str(r, 1)?.unwrap_or_default(),
            project_id: opt_str(r, 2)?.unwrap_or_default(),
            title: opt_str(r, 3)?.unwrap_or_default(),
            content: opt_str(r, 4)?.unwrap_or_default(),
            desc: opt_str(r, 5)?.unwrap_or_default(),
            kind: opt_str(r, 6)?.unwrap_or_else(|| "TEXT".into()),
            status: int(r, 7)?,
            priority: int(r, 8)?,
            start: opt_str(r, 9)?.filter(|s| !s.starts_with("0001")),
            due: opt_str(r, 10)?.filter(|s| !s.starts_with("0001")),
            all_day: int(r, 11)? == 1,
            repeat: opt_str(r, 12)?,
            repeat_from: opt_str(r, 13)?,
            tags: parse_json_list(opt_str(r, 14)?.as_deref()),
            parent_id: opt_str(r, 15)?,
            children: parse_json_list(opt_str(r, 16)?.as_deref()),
            sort_order: int(r, 17)?,
            deleted: int(r, 18)?,
            created: opt_str(r, 19)?,
            modified: opt_str(r, 20)?,
            completed: opt_str(r, 21)?,
            column_id: opt_str(r, 22)?,
            pinned: opt_str(r, 23)?.filter(|s| s != "0" && s != "-1"),
            progress: int(r, 24)?,
            time_zone: opt_str(r, 25)?,
            reminder_count: int(r, 26)?,
            floating: int(r, 27)? == 1,
            user_id: opt_str(r, 28)?,
            etag: opt_str(r, 29)?,
            ex_dates: opt_str(r, 30)?,
            assignee: opt_str(r, 31)?,
        })
    }

    pub fn load_by_sid(conn: &Connection, sid: &str) -> Result<Option<Task>> {
        Ok(conn
            .query_row(
                &format!("SELECT {TASK_COLS} FROM TaskModel WHERE id = ? ORDER BY _Id LIMIT 1"),
                [sid],
                Task::from_row,
            )
            .optional()?)
    }

    pub fn is_repeating(&self) -> bool {
        self.repeat.as_deref().is_some_and(|r| !r.trim().is_empty())
    }
}

pub fn status_name(s: i64) -> &'static str {
    match s {
        0 => "open",
        1 | 2 => "completed",
        -1 => "wontdo",
        _ => "unknown",
    }
}

pub fn priority_name(p: i64) -> &'static str {
    match p {
        5 => "high",
        3 => "medium",
        1 => "low",
        _ => "none",
    }
}

pub fn parse_priority(s: &str) -> Result<i64> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "high" | "h" | "5" | "!!!" => 5,
        "medium" | "med" | "m" | "3" | "!!" => 3,
        "low" | "l" | "1" | "!" => 1,
        "none" | "n" | "0" => 0,
        _ => bail!("Unknown priority '{s}' (use none, low, medium, high)"),
    })
}

pub fn kind_name(k: &str) -> &'static str {
    match k {
        "NOTE" => "note",
        "CHECKLIST" => "checklist",
        _ => "task",
    }
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Project {
    pub local_id: Option<i64>,
    pub sid: String,
    pub name: String,
    pub color: Option<String>,
    pub group_id: Option<String>,
    pub closed: bool,
    pub deleted: bool,
    pub inbox: bool,
    pub kind: Option<String>,
    pub view_mode: Option<String>,
    pub sort_order: i64,
    pub muted: bool,
    pub in_all: bool,
    pub sync_status: Option<String>,
}

pub const PROJECT_COLS: &str = "_Id, id, name, color, groupId, closed, delete_status, Isinbox, kind, \
    viewMode, sortOrder, muted, inAll, sync_status";

impl Project {
    pub fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            local_id: Some(r.get(0)?),
            sid: opt_str(r, 1)?.unwrap_or_default(),
            name: opt_str(r, 2)?.unwrap_or_default(),
            color: opt_str(r, 3)?,
            group_id: opt_str(r, 4)?.filter(|g| g != "NONE"),
            closed: int(r, 5)? == 1,
            deleted: int(r, 6)? == 1,
            inbox: int(r, 7)? == 1,
            kind: opt_str(r, 8)?,
            view_mode: opt_str(r, 9)?,
            sort_order: int(r, 10)?,
            muted: int(r, 11)? == 1,
            in_all: int(r, 12)? == 1,
            sync_status: opt_str(r, 13)?,
        })
    }

    pub fn inbox(inbox_id: &str) -> Self {
        Self {
            local_id: None,
            sid: inbox_id.to_string(),
            name: "Inbox".into(),
            color: None,
            group_id: None,
            closed: false,
            deleted: false,
            inbox: true,
            kind: Some("TASK".into()),
            view_mode: Some("list".into()),
            sort_order: i64::MIN,
            muted: false,
            in_all: true,
            sync_status: None,
        }
    }

    pub fn to_json(&self, conn: &Connection) -> Value {
        let open: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM TaskModel WHERE projectId = ? AND status = 0 AND (deleted = 0 OR deleted IS NULL)",
                [&self.sid],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let folder: Option<String> = self.group_id.as_ref().and_then(|g| {
            conn.query_row(
                "SELECT name FROM ProjectGroupModel WHERE id = ?",
                [g],
                |r| r.get(0),
            )
            .ok()
        });
        json!({
            "id": self.local_id,
            "sid": self.sid,
            "name": self.name,
            "inbox": self.inbox,
            "color": self.color,
            "folder_id": self.group_id,
            "folder": folder,
            "archived": self.closed,
            "kind": self.kind.as_deref().unwrap_or("TASK").to_ascii_lowercase(),
            "view_mode": self.view_mode,
            "muted": self.muted,
            "show_in_all": self.in_all,
            "sort_order": self.sort_order,
            "open_tasks": open,
        })
    }
}

/// All live projects, including a synthetic Inbox if the app never stored one.
pub fn all_projects(
    conn: &Connection,
    inbox_id: &str,
    include_archived: bool,
) -> Result<Vec<Project>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PROJECT_COLS} FROM ProjectModel
         WHERE (delete_status IS NULL OR delete_status = 0)
         ORDER BY Isinbox DESC, sortOrder ASC"
    ))?;
    let mut out: Vec<Project> = stmt
        .query_map([], Project::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    if !include_archived {
        out.retain(|p| !p.closed);
    }
    if !out.iter().any(|p| p.inbox || p.sid == inbox_id) {
        out.insert(0, Project::inbox(inbox_id));
    } else {
        for p in out.iter_mut() {
            if p.sid == inbox_id {
                p.inbox = true;
            }
        }
    }
    Ok(out)
}

/// Resolve a project by "inbox", local _Id, sid, or name (case-insensitive, unique prefix allowed).
pub fn resolve_project(conn: &Connection, inbox_id: &str, key: &str) -> Result<Project> {
    let all = all_projects(conn, inbox_id, true)?;
    if key.eq_ignore_ascii_case("inbox") {
        return Ok(all
            .into_iter()
            .find(|p| p.inbox)
            .unwrap_or_else(|| Project::inbox(inbox_id)));
    }
    if let Ok(n) = key.parse::<i64>()
        && let Some(p) = all.iter().find(|p| p.local_id == Some(n))
    {
        return Ok(p.clone());
    }
    if let Some(p) = all.iter().find(|p| p.sid == key) {
        return Ok(p.clone());
    }
    if let Some(p) = all.iter().find(|p| p.name == key) {
        return Ok(p.clone());
    }
    let lower = key.to_lowercase();
    let exact: Vec<&Project> = all
        .iter()
        .filter(|p| p.name.to_lowercase() == lower)
        .collect();
    if exact.len() == 1 {
        return Ok(exact[0].clone());
    }
    let prefix: Vec<&Project> = all
        .iter()
        .filter(|p| p.name.to_lowercase().starts_with(&lower))
        .collect();
    match prefix.len() {
        1 => Ok(prefix[0].clone()),
        0 => Err(anyhow!("Project not found: {key}")),
        _ => Err(anyhow!(
            "Project '{key}' is ambiguous: {}",
            prefix
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub fn project_name(conn: &Connection, inbox_id: &str, sid: &str) -> String {
    if sid == inbox_id {
        return "Inbox".into();
    }
    conn.query_row("SELECT name FROM ProjectModel WHERE id = ?", [sid], |r| {
        r.get(0)
    })
    .unwrap_or_else(|_| "(unknown)".into())
}

// ---------------------------------------------------------------------------
// Task resolution
// ---------------------------------------------------------------------------

/// Resolve a task by local _Id, sid, unique sid prefix (>=6 chars), or exact title.
pub fn resolve_task(conn: &Connection, user_id: &str, key: &str) -> Result<Task> {
    let key = key.trim();
    if let Ok(n) = key.parse::<i64>()
        && let Some(t) = conn
            .query_row(
                &format!("SELECT {TASK_COLS} FROM TaskModel WHERE _Id = ?"),
                [n],
                Task::from_row,
            )
            .optional()?
    {
        return Ok(t);
    }
    if let Some(t) = Task::load_by_sid(conn, key)? {
        return Ok(t);
    }
    if key.len() >= 6 && key.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut stmt = conn.prepare(&format!(
            "SELECT {TASK_COLS} FROM TaskModel WHERE id LIKE ? AND (userId = ? OR userId IS NULL)"
        ))?;
        let hits: Vec<Task> = stmt
            .query_map(params![format!("{key}%"), user_id], Task::from_row)?
            .collect::<rusqlite::Result<_>>()?;
        if hits.len() == 1 {
            return Ok(hits.into_iter().next().unwrap());
        }
        if hits.len() > 1 {
            bail!("Task id prefix '{key}' matches {} tasks", hits.len());
        }
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT {TASK_COLS} FROM TaskModel WHERE title = ? COLLATE NOCASE AND (deleted IS NULL OR deleted <> 2)"
    ))?;
    let all: Vec<Task> = stmt
        .query_map([key], Task::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    // Prefer open tasks, then other live tasks, then trashed ones.
    let tiers: [&dyn Fn(&Task) -> bool; 3] = [
        &|t| t.deleted == 0 && t.status == 0,
        &|t| t.deleted == 0,
        &|_| true,
    ];
    let hits: Vec<Task> = tiers
        .iter()
        .map(|f| all.iter().filter(|t| f(t)).cloned().collect::<Vec<_>>())
        .find(|v| !v.is_empty())
        .unwrap_or_default();
    match hits.len() {
        1 => Ok(hits.into_iter().next().unwrap()),
        0 => Err(anyhow!("Task not found: {key}")),
        n => Err(anyhow!(
            "Title '{key}' matches {n} tasks (ids: {}); use an id",
            hits.iter()
                .map(|t| t.local_id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

// ---------------------------------------------------------------------------
// Checklist items and reminders
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Item {
    pub local_id: i64,
    pub sid: String,
    pub task_sid: String,
    pub title: String,
    pub status: i64,
    pub sort_order: i64,
    pub start: Option<String>,
    pub all_day: bool,
    pub completed: Option<String>,
}

pub const ITEM_COLS: &str =
    "_Id, id, TaskServerId, title, status, sortOrder, startDate, isAllDay, completedTime";

impl Item {
    pub fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            local_id: r.get(0)?,
            sid: opt_str(r, 1)?.unwrap_or_default(),
            task_sid: opt_str(r, 2)?.unwrap_or_default(),
            title: opt_str(r, 3)?.unwrap_or_default(),
            status: int(r, 4)?,
            sort_order: int(r, 5)?,
            start: opt_str(r, 6)?.filter(|s| !s.starts_with("0001")),
            all_day: int(r, 7)? == 1,
            completed: opt_str(r, 8)?,
        })
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.local_id,
            "sid": self.sid,
            "task_sid": self.task_sid,
            "title": self.title,
            "done": self.status == 1,
            "sort_order": self.sort_order,
            "date": self.start,
            "all_day": self.all_day,
            "completed_time": self.completed,
        })
    }
}

pub fn task_items(conn: &Connection, task_sid: &str) -> Result<Vec<Item>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ITEM_COLS} FROM TaskDetailItemModel WHERE TaskServerId = ? ORDER BY sortOrder, _Id"
    ))?;
    Ok(stmt
        .query_map([task_sid], Item::from_row)?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn resolve_item(conn: &Connection, key: &str, task_sid: Option<&str>) -> Result<Item> {
    if let Some(task) = task_sid {
        let items = task_items(conn, task)?;
        // 1-based position within the task's checklist
        if let Ok(pos) = key.parse::<usize>()
            && pos >= 1
            && pos <= items.len()
        {
            return Ok(items[pos - 1].clone());
        }
        if let Some(i) = items.iter().find(|i| i.sid == key || i.title == key) {
            return Ok(i.clone());
        }
        let lower = key.to_lowercase();
        let hits: Vec<&Item> = items
            .iter()
            .filter(|i| i.title.to_lowercase().contains(&lower))
            .collect();
        if hits.len() == 1 {
            return Ok(hits[0].clone());
        }
        bail!("Checklist item not found in task: {key}");
    }
    let by_local = key.parse::<i64>().ok();
    conn.query_row(
        &format!("SELECT {ITEM_COLS} FROM TaskDetailItemModel WHERE id = ? OR _Id = ? LIMIT 1"),
        params![key, by_local.unwrap_or(-1)],
        Item::from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow!("Checklist item not found: {key}"))
}

pub fn task_reminders(conn: &Connection, task_sid: &str) -> Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT trigger FROM TaskReminderModel WHERE taskserverid = ? ORDER BY _Id")?;
    Ok(stmt
        .query_map([task_sid], |r| r.get::<_, Option<String>>(0))?
        .filter_map(|r| r.ok().flatten())
        .collect())
}

/// Human-friendly description of a reminder trigger.
pub fn describe_trigger(t: &str) -> String {
    let body = t.trim_start_matches("TRIGGER:");
    if body == "PT0S" || body == "-PT0S" {
        return "on time".into();
    }
    body.to_string()
}

// ---------------------------------------------------------------------------
// Task JSON
// ---------------------------------------------------------------------------

pub fn task_json(conn: &Connection, inbox_id: &str, t: &Task, full: bool) -> Value {
    let mut v = json!({
        "id": t.local_id,
        "sid": t.sid,
        "title": t.title,
        "status": status_name(t.status),
        "priority": priority_name(t.priority),
        "kind": kind_name(&t.kind),
        "project_sid": t.project_id,
        "project": project_name(conn, inbox_id, &t.project_id),
        "tags": t.tags,
        "start": t.start,
        "due": t.due,
        "all_day": t.all_day,
        "repeat": t.repeat.as_ref().filter(|r| !r.is_empty()),
        "parent_sid": t.parent_id.as_ref().filter(|p| !p.is_empty()),
        "pinned": t.pinned.is_some(),
        "trashed": t.deleted != 0,
        "created_time": t.created,
        "modified_time": t.modified,
        "completed_time": t.completed,
        "sort_order": t.sort_order,
    });
    if full {
        let items: Vec<Value> = task_items(conn, &t.sid)
            .unwrap_or_default()
            .iter()
            .map(Item::to_json)
            .collect();
        let reminders = task_reminders(conn, &t.sid).unwrap_or_default();
        let children: Vec<Value> = {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {TASK_COLS} FROM TaskModel WHERE parentId = ? AND (deleted = 0 OR deleted IS NULL) ORDER BY sortOrder"
                ))
                .ok();
            stmt.as_mut()
                .and_then(|s| s.query_map([&t.sid], Task::from_row).ok().map(|rows| {
                    rows.filter_map(|r| r.ok())
                        .map(|c| json!({"id": c.local_id, "sid": c.sid, "title": c.title, "status": status_name(c.status)}))
                        .collect()
                }))
                .unwrap_or_default()
        };
        let column: Option<String> = t.column_id.as_ref().and_then(|c| {
            conn.query_row("SELECT name FROM ColumnModel WHERE id = ?", [c], |r| {
                r.get(0)
            })
            .ok()
        });
        let m = v.as_object_mut().unwrap();
        m.insert(
            "content".into(),
            json!(if t.kind == "CHECKLIST" {
                &t.desc
            } else {
                &t.content
            }),
        );
        m.insert("raw_content".into(), json!(t.content));
        m.insert("desc".into(), json!(t.desc));
        m.insert("items".into(), json!(items));
        m.insert("reminders".into(), json!(reminders));
        m.insert("repeat_from".into(), json!(t.repeat_from));
        m.insert("time_zone".into(), json!(t.time_zone));
        m.insert("progress".into(), json!(t.progress));
        m.insert("column_sid".into(), json!(t.column_id));
        m.insert("column".into(), json!(column));
        m.insert("subtasks".into(), json!(children));
        m.insert("ex_dates".into(), json!(t.ex_dates));
    }
    v
}

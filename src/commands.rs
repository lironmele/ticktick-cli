use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};
use std::collections::HashMap;
use tabled::Table;
use tabled::Tabled;
use tabled::settings::Alignment;
use tabled::settings::Style;

#[derive(Tabled)]
struct ProjectRow {
    #[tabled(rename = "ID")]
    id: i64,
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Type")]
    type_: String,
}

pub fn list_projects(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT _Id, name, isinbox
         FROM ProjectModel
         WHERE delete_status IS NULL OR delete_status = 0
         ORDER BY sortOrder ASC",
    )?;

    let projects: Vec<ProjectRow> = stmt
        .query_map([], |row| {
            let id: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            let isinbox: i32 = row.get(2)?;
            let type_ = if isinbox == 1 {
                "(inbox)".to_string()
            } else {
                String::new()
            };
            Ok(ProjectRow { id, name, type_ })
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut table = Table::new(&projects);
    table.with(Alignment::left());
    table.with(Style::psql());

    println!("{}", table);

    Ok(())
}

#[derive(Tabled)]
struct TagRow {
    #[tabled(rename = "Name")]
    name: String,
}

pub fn list_tags(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT name, sortOrder
         FROM TagModel
         ORDER BY sortOrder ASC",
    )?;

    let tags: Vec<TagRow> = stmt
        .query_map([], |row| Ok(TagRow { name: row.get(0)? }))?
        .filter_map(|r| r.ok())
        .collect();

    if tags.is_empty() {
        println!("No tags found.");
        return Ok(());
    }

    let mut table = Table::new(&tags);
    table.with(Alignment::left());
    table.with(Style::psql());

    println!("{}", table);

    Ok(())
}

#[derive(Tabled)]
struct PomoDisplayRow {
    #[tabled(rename = "Start")]
    start: String,
    #[tabled(rename = "End")]
    end: String,
    #[tabled(rename = "Duration")]
    duration: String,
    #[tabled(rename = "Task")]
    task: String,
    #[tabled(rename = "Project")]
    project: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Type")]
    type_: String,
    #[tabled(rename = "Note")]
    note: String,
}

#[derive(Tabled)]
struct PomoSummaryRow {
    #[tabled(rename = "Date")]
    date: String,
    #[tabled(rename = "Pomos")]
    pomos: i64,
    #[tabled(rename = "Sessions")]
    sessions: i64,
    #[tabled(rename = "Duration")]
    duration: String,
}

pub fn list_pomos(
    conn: &Connection,
    date: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    task: Option<&str>,
    project: Option<&str>,
    summary: bool,
    limit: usize,
) -> Result<()> {
    let filters = PomoFilters::new(conn, date, from, to, task, project)?;
    if summary {
        list_pomo_summary(conn, &filters)
    } else {
        list_pomo_records(conn, &filters, limit)
    }
}

fn list_pomo_records(conn: &Connection, filters: &PomoFilters, limit: usize) -> Result<()> {
    let mut sql = format!(
        "SELECT
             COALESCE(pt.PomoId, p.Id) AS pomo_id,
             COALESCE(NULLIF(pt.TaskId, ''), NULLIF(p.TaskSid, '')) AS task_id,
             COALESCE(NULLIF(pt.Title, ''), tm.title, '') AS task_title,
             COALESCE(NULLIF(pt.ProjectName, ''), pm.name, '') AS project_name,
             COALESCE(NULLIF(pt.StartTime, ''), p.StartTime) AS start_time,
             COALESCE(NULLIF(pt.EndTime, ''), p.EndTime) AS end_time,
             p.Status,
             p.Type,
             COALESCE(p.PauseDuration, 0),
             COALESCE(NULLIF(p.Note, ''), '') AS note
         FROM PomodoroModel p
         LEFT JOIN PomoTask pt ON pt.PomoId = p.Id
         LEFT JOIN TaskModel tm ON tm.id = COALESCE(NULLIF(pt.TaskId, ''), NULLIF(p.TaskSid, ''))
         LEFT JOIN ProjectModel pm ON pm.id = tm.projectId
         {}",
        filters.where_clause
    );
    sql.push_str(" ORDER BY datetime(start_time) DESC, pomo_id DESC LIMIT ?");

    let mut params = filters.params.clone();
    params.push(Value::Integer(limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        let start: Option<String> = row.get(4)?;
        let end: Option<String> = row.get(5)?;
        let seconds = duration_seconds(start.as_deref(), end.as_deref());
        Ok(PomoDisplayRow {
            start: format_datetime(start.as_deref()),
            end: format_time(end.as_deref()),
            duration: format_duration(seconds),
            task: display_text(row.get::<_, Option<String>>(2)?.as_deref()),
            project: display_text(row.get::<_, Option<String>>(3)?.as_deref()),
            status: pomo_status_text(row.get::<_, Option<i32>>(6)?).to_string(),
            type_: pomo_type_text(row.get::<_, Option<i32>>(7)?).to_string(),
            note: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
        })
    })?;

    let pomos: Vec<PomoDisplayRow> = rows.filter_map(|r| r.ok()).collect();
    if pomos.is_empty() {
        println!("No pomodoro records found.");
        return Ok(());
    }

    let mut table = Table::new(&pomos);
    table.with(Alignment::left());
    table.with(Style::psql());
    println!("{}", table);

    Ok(())
}

fn list_pomo_summary(conn: &Connection, filters: &PomoFilters) -> Result<()> {
    let sql = format!(
        "SELECT
             date(COALESCE(NULLIF(pt.StartTime, ''), p.StartTime)) AS day,
             COUNT(DISTINCT p.Id) AS pomos,
             COUNT(*) AS sessions,
             SUM(
                 strftime('%s', COALESCE(NULLIF(pt.EndTime, ''), p.EndTime)) -
                 strftime('%s', COALESCE(NULLIF(pt.StartTime, ''), p.StartTime))
             ) AS duration_seconds
         FROM PomodoroModel p
         LEFT JOIN PomoTask pt ON pt.PomoId = p.Id
         LEFT JOIN TaskModel tm ON tm.id = COALESCE(NULLIF(pt.TaskId, ''), NULLIF(p.TaskSid, ''))
         LEFT JOIN ProjectModel pm ON pm.id = tm.projectId
         {}
         GROUP BY day
         ORDER BY day DESC",
        filters.where_clause
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(filters.params.iter()), |row| {
        Ok(PomoSummaryRow {
            date: row.get::<_, String>(0)?,
            pomos: row.get(1)?,
            sessions: row.get(2)?,
            duration: format_duration(row.get::<_, Option<i64>>(3)?.unwrap_or_default()),
        })
    })?;

    let summaries: Vec<PomoSummaryRow> = rows.filter_map(|r| r.ok()).collect();
    if summaries.is_empty() {
        println!("No pomodoro records found.");
        return Ok(());
    }

    let mut table = Table::new(&summaries);
    table.with(Alignment::left());
    table.with(Style::psql());
    println!("{}", table);

    Ok(())
}

#[derive(Tabled)]
struct TaskDisplayRow {
    #[tabled(rename = "ID")]
    id: i64,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Priority")]
    priority: String,
    #[tabled(rename = "Title")]
    title: String,
    #[tabled(rename = "Project")]
    project: String,
    #[tabled(rename = "Tags")]
    tags: String,
}

pub fn list_tasks(
    conn: &Connection,
    project: Option<&str>,
    show_all: bool,
    tags_filter: Option<&str>,
) -> Result<()> {
    let project_filter = if let Some(p) = project {
        if p.eq_ignore_ascii_case("inbox") {
            conn.query_row(
                "SELECT id FROM ProjectModel WHERE isinbox = 1 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok()
        } else if let Ok(num_id) = p.parse::<i64>() {
            let project_uuid: Option<String> = conn
                .query_row(
                    "SELECT id FROM ProjectModel WHERE _Id = ?",
                    [num_id],
                    |row| row.get(0),
                )
                .ok();
            project_uuid
        } else {
            Some(p.to_string())
        }
    } else {
        None
    };

    let status_filter = if show_all {
        "AND (deleted = 0 OR deleted IS NULL)"
    } else {
        "AND status = 0 AND (deleted = 0 OR deleted IS NULL)"
    };

    let mut proj_stmt = conn.prepare("SELECT id, name FROM ProjectModel")?;
    let project_names: HashMap<String, String> = proj_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let query = format!(
        "SELECT _Id, projectId, title, status, priority, dueDate, startDate, createdTime, modifiedTime, tag
         FROM TaskModel
         WHERE projectId LIKE ? {}",
        status_filter
    );

    let mut stmt = conn.prepare(&query)?;

    let project_id = project_filter.as_deref().unwrap_or("%");
    let mut rows = stmt.query([project_id])?;

    let mut tasks = Vec::new();
    while let Some(row) = rows.next()? {
        tasks.push(TaskRowInternal {
            id: row.get(0)?,
            project_id: row.get(1)?,
            title: row.get(2)?,
            status: row.get(3)?,
            priority: row.get(4)?,
            tag: row.get_unwrap(9),
            due_date: row.get_unwrap(5),
            start_date: row.get_unwrap(6),
            created_time: row.get_unwrap(7),
            modified_time: row.get_unwrap(8),
        });
    }

    if tasks.is_empty() {
        println!("No tasks found.");
        return Ok(());
    }

    let filter_tag_names: Vec<String> = tags_filter
        .map(|s| s.split(',').map(str::trim).map(String::from).collect())
        .unwrap_or_default();

    let filtered_tasks: Vec<&TaskRowInternal> = if !filter_tag_names.is_empty() {
        tasks
            .iter()
            .filter(|t| {
                let task_tag_names = parse_tags(t.tag.as_deref());
                filter_tag_names
                    .iter()
                    .any(|ft| task_tag_names.contains(ft))
            })
            .collect()
    } else {
        tasks.iter().collect()
    };

    if filtered_tasks.is_empty() {
        println!("No tasks found.");
        return Ok(());
    }

    let display_tasks: Vec<TaskDisplayRow> = filtered_tasks
        .iter()
        .map(|t| {
            let status_icon = match t.status {
                0 => "[ ]",
                2 => "[x]",
                -1 => "[-]",
                _ => "[?]",
            };
            let priority_icon = match t.priority {
                3 => "High",
                2 => "Medium",
                1 => "Low",
                _ => "None",
            };
            let project_name = project_names
                .get(&t.project_id)
                .map(|s| s.as_str())
                .unwrap_or("Unknown");

            TaskDisplayRow {
                id: t.id,
                status: status_icon.to_string(),
                priority: priority_icon.to_string(),
                title: t.title.clone(),
                project: project_name.to_string(),
                tags: format_tags(t.tag.as_deref()),
            }
        })
        .collect();

    let mut table = Table::new(&display_tasks);
    table.with(Alignment::left());
    table.with(Style::psql());

    println!("{}", table);

    Ok(())
}

pub fn show_task(conn: &Connection, id: i64) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT _Id, projectId, title, content, status, priority, dueDate, startDate,
                createdTime, modifiedTime, reminder, isAllDay, progress, kind, tag
         FROM TaskModel
         WHERE _Id = ?",
    )?;

    let task = stmt.query_row([id], |row| {
        Ok(TaskDetail {
            id: row.get(0)?,
            project_id: row.get(1)?,
            title: row.get(2)?,
            content: row.get(3)?,
            status: row.get(4)?,
            priority: row.get(5)?,
            due_date: row.get_unwrap(6),
            start_date: row.get_unwrap(7),
            created_time: row.get_unwrap(8),
            modified_time: row.get_unwrap(9),
            reminder: row.get_unwrap(10),
            is_all_day: row.get_unwrap(11),
            progress: row.get_unwrap(12),
            kind: row.get_unwrap(13),
            tag: row.get_unwrap(14),
        })
    });

    match task {
        Ok(t) => {
            let project_name: String = conn
                .query_row(
                    "SELECT name FROM ProjectModel WHERE id = ?",
                    [&t.project_id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "Unknown".to_string());

            println!("Task: {}", t.title);
            println!("{}", "=".repeat(60));
            println!("ID:         {}", t.id);
            println!(
                "Project:    {} ({})",
                project_name,
                shorten_id(&t.project_id)
            );
            println!("Status:     {}", status_text(t.status));
            println!("Priority:   {}", priority_text(t.priority));
            if let Some(kind) = &t.kind {
                println!("Kind:       {}", kind);
            }

            if let Some(ref due) = t.due_date {
                if !due.is_empty() {
                    println!("Due:        {}", due);
                }
            }
            if let Some(ref start) = t.start_date {
                if !start.is_empty() {
                    println!("Start:      {}", start);
                }
            }
            if let Some(ref reminder) = t.reminder {
                if !reminder.is_empty() {
                    println!("Reminder:   {}", reminder);
                }
            }
            if t.is_all_day == Some(1) {
                println!("All-Day:    Yes");
            }
            let tags_str = format_tags(t.tag.as_deref());
            if !tags_str.is_empty() {
                println!("Tags:       {}", tags_str);
            }
            if let Some(progress) = t.progress {
                if progress > 0 {
                    println!("Progress:   {}%", progress);
                }
            }

            println!("\nContent:");
            println!(
                "{}",
                if t.content.is_empty() {
                    "(none)".to_string()
                } else {
                    t.content
                }
            );

            if let Some(ref created) = t.created_time {
                println!("\nCreated:    {}", created);
            }
            if let Some(ref modified) = t.modified_time {
                println!("Modified:   {}", modified);
            }

            Ok(())
        }
        Err(e) => {
            anyhow::bail!("Task query failed: {} (id={})", e, id);
        }
    }
}

struct TaskRowInternal {
    id: i64,
    project_id: String,
    title: String,
    status: i32,
    priority: i32,
    tag: Option<String>,
    #[allow(dead_code)]
    due_date: Option<String>,
    #[allow(dead_code)]
    start_date: Option<String>,
    #[allow(dead_code)]
    created_time: Option<String>,
    #[allow(dead_code)]
    modified_time: Option<String>,
}

struct TaskDetail {
    id: i64,
    project_id: String,
    title: String,
    content: String,
    status: i32,
    priority: i32,
    due_date: Option<String>,
    start_date: Option<String>,
    created_time: Option<String>,
    modified_time: Option<String>,
    reminder: Option<String>,
    is_all_day: Option<i32>,
    progress: Option<i32>,
    kind: Option<String>,
    tag: Option<String>,
}

fn parse_tags(tag_json: Option<&str>) -> Vec<String> {
    match tag_json {
        Some(json) => serde_json::from_str::<Vec<String>>(json).unwrap_or_default(),
        None => Vec::new(),
    }
}

fn format_tags(tag_json: Option<&str>) -> String {
    parse_tags(tag_json).join(", ")
}

fn shorten_id(id: &str) -> String {
    if id.chars().count() > 8 {
        let mut chars = id.chars();
        let result: String = chars.by_ref().take(8).collect();
        format!("{}...", result)
    } else {
        id.to_string()
    }
}

fn status_text(status: i32) -> &'static str {
    match status {
        0 => "Pending",
        2 => "Completed",
        -1 => "Cancelled",
        _ => "Unknown",
    }
}

fn priority_text(priority: i32) -> &'static str {
    match priority {
        3 => "High 🔴",
        2 => "Medium 🟡",
        1 => "Low 🟢",
        _ => "None ⚪",
    }
}

struct PomoFilters {
    where_clause: String,
    params: Vec<Value>,
}

impl PomoFilters {
    fn new(
        conn: &Connection,
        date: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        task: Option<&str>,
        project: Option<&str>,
    ) -> Result<Self> {
        let start_expr = "COALESCE(NULLIF(pt.StartTime, ''), p.StartTime)";
        let task_expr = "COALESCE(NULLIF(pt.TaskId, ''), NULLIF(p.TaskSid, ''))";
        let project_expr = "COALESCE(NULLIF(pt.ProjectName, ''), pm.name, '')";

        let mut conditions = vec![format!("{start_expr} IS NOT NULL")];
        let mut params = Vec::new();

        if let Some(date) = date {
            validate_date(date, "date")?;
            conditions.push(format!("date({start_expr}) = date(?)"));
            params.push(Value::Text(date.to_string()));
        }
        if let Some(from) = from {
            validate_date(from, "from")?;
            conditions.push(format!("date({start_expr}) >= date(?)"));
            params.push(Value::Text(from.to_string()));
        }
        if let Some(to) = to {
            validate_date(to, "to")?;
            conditions.push(format!("date({start_expr}) <= date(?)"));
            params.push(Value::Text(to.to_string()));
        }
        if let Some(task) = task {
            let task_sid = resolve_task_sid(conn, task)?;
            conditions.push(format!("{task_expr} = ?"));
            params.push(Value::Text(task_sid));
        }
        if let Some(project) = project {
            let project_filter = resolve_project_filter(conn, project)?;
            conditions.push(format!(
                "(tm.projectId = ? OR {project_expr} = ? OR {project_expr} LIKE ?)"
            ));
            params.push(Value::Text(project_filter.id));
            params.push(Value::Text(project_filter.name.clone()));
            params.push(Value::Text(format!("%{}%", project_filter.name)));
        }

        Ok(Self {
            where_clause: format!("WHERE {}", conditions.join(" AND ")),
            params,
        })
    }
}

struct ProjectFilter {
    id: String,
    name: String,
}

fn resolve_task_sid(conn: &Connection, task: &str) -> Result<String> {
    if let Ok(local_id) = task.parse::<i64>() {
        return conn
            .query_row(
                "SELECT id FROM TaskModel WHERE _Id = ?",
                [local_id],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("Task not found: {}", task));
    }

    Ok(task.to_string())
}

fn resolve_project_filter(conn: &Connection, project: &str) -> Result<ProjectFilter> {
    let row: Option<(String, String)> = if project.eq_ignore_ascii_case("inbox") {
        conn.query_row(
            "SELECT id, name FROM ProjectModel WHERE isinbox = 1 LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()
    } else if let Ok(local_id) = project.parse::<i64>() {
        conn.query_row(
            "SELECT id, name FROM ProjectModel WHERE _Id = ?",
            [local_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()
    } else {
        conn.query_row(
            "SELECT id, name FROM ProjectModel WHERE id = ? OR name = ? LIMIT 1",
            [project, project],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()
    };

    Ok(match row {
        Some((id, name)) => ProjectFilter { id, name },
        None => ProjectFilter {
            id: project.to_string(),
            name: project.to_string(),
        },
    })
}

fn validate_date(value: &str, name: &str) -> Result<()> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|_| anyhow::anyhow!("Invalid --{} date: {} (expected YYYY-MM-DD)", name, value))
}

fn duration_seconds(start: Option<&str>, end: Option<&str>) -> i64 {
    match (start, end) {
        (Some(start), Some(end)) => {
            let start = parse_ticktick_datetime(start);
            let end = parse_ticktick_datetime(end);
            match (start, end) {
                (Some(start), Some(end)) => (end - start).num_seconds().max(0),
                _ => 0,
            }
        }
        _ => 0,
    }
}

fn parse_ticktick_datetime(value: &str) -> Option<chrono::NaiveDateTime> {
    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").ok()
}

fn format_datetime(value: Option<&str>) -> String {
    value
        .and_then(parse_ticktick_datetime)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn format_time(value: Option<&str>) -> String {
    value
        .and_then(parse_ticktick_datetime)
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_default()
}

fn format_duration(seconds: i64) -> String {
    if seconds <= 0 {
        return "0m".to_string();
    }

    let minutes = (seconds + 30) / 60;
    let hours = minutes / 60;
    let mins = minutes % 60;
    if hours > 0 && mins > 0 {
        format!("{}h {}m", hours, mins)
    } else if hours > 0 {
        format!("{}h", hours)
    } else {
        format!("{}m", mins)
    }
}

fn display_text(value: Option<&str>) -> String {
    match value {
        Some(value) if !value.trim().is_empty() => value.to_string(),
        _ => "(none)".to_string(),
    }
}

fn pomo_status_text(status: Option<i32>) -> &'static str {
    match status {
        Some(1) => "Completed",
        Some(0) => "Unfinished",
        _ => "Unknown",
    }
}

fn pomo_type_text(type_: Option<i32>) -> &'static str {
    match type_ {
        Some(0) => "Pomodoro",
        Some(1) => "Stopwatch",
        _ => "Unknown",
    }
}

use anyhow::Result;
use rusqlite::Connection;
use std::collections::HashMap;

const INBOX_ID: &str = "inbox1020477006";

pub fn list_projects(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT _Id, name, color, sortOrder, isinbox
         FROM ProjectModel
         WHERE delete_status IS NULL OR delete_status = 0
         ORDER BY sortOrder ASC"
    )?;

    let projects: Vec<(i64, String, String, i64, bool)> = stmt
        .query_map([], |row| {
            let id: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            let color: String = row.get(2)?;
            let sort_order: i64 = row.get(3)?;
            let isinbox: i32 = row.get(4)?;
            Ok((id, name, color, sort_order, isinbox == 1))
        })?
        .filter_map(|r| r.ok())
        .collect();

    println!("{:<10} {:<30} {}", "ID", "Name", "Color");
    println!("{}", "-".repeat(60));

    for (id, name, color, _, isinbox) in projects {
        if isinbox {
            println!("{:<10} {:<30} (inbox)", id, name);
        } else {
            println!("{:<10} {:<30} {}", id, name, color);
        }
    }

    Ok(())
}

pub fn list_tags(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, name, color, sortOrder
         FROM TagModel
         WHERE deleted IS NULL OR deleted = 0
         ORDER BY sortOrder ASC"
    )?;

    let tags: Vec<(String, String, String, i64)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

    if tags.is_empty() {
        println!("No tags found.");
        return Ok(());
    }

    println!("{:<20} {:<30} {}", "ID", "Name", "Color");
    println!("{}", "-".repeat(60));

    for (id, name, color, _) in tags {
        println!("{:<20} {:<30} {}", shorten_id(&id), name, color);
    }

    Ok(())
}

pub fn list_tasks(conn: &Connection, project: Option<&str>, pending: bool, completed: bool) -> Result<()> {
    let project_filter = if let Some(p) = project {
        if p.eq_ignore_ascii_case("inbox") {
            Some(INBOX_ID.to_string())
        } else {
            Some(p.to_string())
        }
    } else {
        None
    };

    let status_filter = if pending && !completed {
        "AND status = 0 AND (deleted = 0 OR deleted IS NULL)"
    } else if completed && !pending {
        "AND status = 2 AND (deleted = 0 OR deleted IS NULL)"
    } else {
        "AND (deleted = 0 OR deleted IS NULL)"
    };

    let mut proj_stmt = conn.prepare("SELECT id, name FROM ProjectModel")?;
    let project_names: HashMap<String, String> = proj_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let query = format!(
        "SELECT _Id, projectId, title, status, priority, dueDate, startDate, createdTime, modifiedTime
         FROM TaskModel
         WHERE projectId LIKE ? {}",
        status_filter
    );

    let mut stmt = conn.prepare(&query)?;

    let project_id = project_filter.as_deref().unwrap_or("%");
    let mut rows = stmt.query([project_id])?;

    let mut tasks = Vec::new();
    while let Some(row) = rows.next()? {
        tasks.push(TaskRow {
            id: row.get(0)?,
            project_id: row.get(1)?,
            title: row.get(2)?,
            status: row.get(3)?,
            priority: row.get(4)?,
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

    println!("{:<6} {:<8} {:<5} {:<35} {:<20}", "ID", "Status", "Pri", "Title", "Project");
    println!("{}", "-".repeat(80));

    for task in tasks {
        let status_icon = match task.status {
            0 => "[ ]",
            2 => "[x]",
            -1 => "[-]",
            _ => "[?]",
        };

        let priority_icon = match task.priority {
            3 => "🔴",
            2 => "🟡",
            1 => "🟢",
            _ => "⚪",
        };

        let project_name = project_names
            .get(&task.project_id)
            .map(|s| s.as_str())
            .unwrap_or("Unknown");

        println!(
            "{:<6} {} {:<5} {:<35} {:<20}",
            task.id,
            status_icon,
            priority_icon,
            truncate(&task.title, 35),
            truncate(project_name, 20)
        );
    }

    Ok(())
}

pub fn show_task(conn: &Connection, id: i64) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT _Id, projectId, title, content, status, priority, dueDate, startDate,
                createdTime, modifiedTime, reminder, isAllDay, progress, kind, tag
         FROM TaskModel
         WHERE _Id = ?"
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
            let project_name: String = conn.query_row(
                "SELECT name FROM ProjectModel WHERE id = ?",
                [&t.project_id],
                |row| row.get(0),
            ).unwrap_or_else(|_| "Unknown".to_string());

            println!("Task: {}", t.title);
            println!("{}", "=".repeat(60));
            println!("ID:         {}", t.id);
            println!("Project:    {} ({})", project_name, shorten_id(&t.project_id));
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
            if let Some(ref tag) = t.tag {
                if !tag.is_empty() {
                    println!("Tags:       {}", tag);
                }
            }
            if let Some(progress) = t.progress {
                if progress > 0 {
                    println!("Progress:   {}%", progress);
                }
            }

            println!("\nContent:");
            println!("{}", if t.content.is_empty() { "(none)".to_string() } else { t.content });

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

struct TaskRow {
    id: i64,
    project_id: String,
    title: String,
    status: i32,
    priority: i32,
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

fn shorten_id(id: &str) -> String {
    if id.chars().count() > 8 {
        let mut chars = id.chars();
        let result: String = chars.by_ref().take(8).collect();
        format!("{}...", result)
    } else {
        id.to_string()
    }
}

fn truncate(s: &str, len: usize) -> String {
    if s.chars().count() > len {
        let mut chars = s.chars();
        let result: String = chars.by_ref().take(len - 3).collect();
        format!("{}...", result)
    } else {
        s.to_string()
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
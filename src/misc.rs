//! Info, agenda, export, raw SQL, schema, backup, comments and smart-list filters.

use crate::ctx::{self, Ctx};
use crate::model::{self, TASK_COLS, Task};
use crate::out;
use crate::tasks;
use anyhow::{Result, bail};
use rusqlite::types::ValueRef;
use serde_json::{Map, Value, json};
use std::path::Path;

pub fn info(ctx: &Ctx) -> Result<()> {
    let count = |sql: &str| -> i64 { ctx.conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1) };
    let username: Option<String> = ctx
        .conn
        .query_row(
            "SELECT username FROM UserModel WHERE userId = ?",
            [&ctx.user_id],
            |r| r.get(0),
        )
        .ok();
    let pending: i64 = ctx
        .conn
        .query_row(
            "SELECT COUNT(*) FROM SyncStatusModel WHERE UserId = ?",
            [&ctx.user_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let v = json!({
        "db": ctx.db_path.display().to_string(),
        "user_id": ctx.user_id,
        "username": username,
        "inbox_sid": ctx.inbox_id,
        "time_zone": ctx.time_zone,
        "app_running": crate::app::is_running(),
        "open_tasks": count("SELECT COUNT(*) FROM TaskModel WHERE status = 0 AND (deleted = 0 OR deleted IS NULL)"),
        "completed_tasks": count("SELECT COUNT(*) FROM TaskModel WHERE status <> 0 AND (deleted = 0 OR deleted IS NULL)"),
        "trashed_tasks": count("SELECT COUNT(*) FROM TaskModel WHERE deleted = 1"),
        "projects": count("SELECT COUNT(*) FROM ProjectModel WHERE (delete_status IS NULL OR delete_status = 0)"),
        "tags": count("SELECT COUNT(*) FROM TagModel WHERE (deleted IS NULL OR deleted = 0)"),
        "habits": count("SELECT COUNT(*) FROM HabitModel WHERE Status >= 0"),
        "pending_sync_changes": pending,
    });
    if ctx.json {
        out::json(&v);
    } else {
        for (k, val) in v.as_object().unwrap() {
            println!("{k:<22}{}", out::s(val));
        }
    }
    Ok(())
}

/// Overdue, today and the next few days in one call: the agent's "what's on my plate".
pub fn agenda(ctx: &Ctx, days: i64) -> Result<()> {
    let open = tasks::query(
        ctx,
        &tasks::ListArgs {
            sort: tasks::SortBy::Date,
            ..Default::default()
        },
    )?;
    let today = crate::dates::today();
    let mut overdue = Vec::new();
    let mut by_day: Vec<(chrono::NaiveDate, Vec<Value>)> = (0..days.max(1))
        .map(|i| (today + chrono::Duration::days(i), Vec::new()))
        .collect();
    let mut undated = 0;
    for t in &open {
        let Some(start) = t.start.as_deref().and_then(crate::dates::parse_db_dt) else {
            undated += 1;
            continue;
        };
        let v = model::task_json(&ctx.conn, &ctx.inbox_id, t, false);
        let d = start.date();
        if d < today {
            overdue.push(v);
        } else if let Some(slot) = by_day.iter_mut().find(|(day, _)| *day == d) {
            slot.1.push(v);
        }
    }
    let days_json: Vec<Value> = by_day
        .iter()
        .map(|(d, ts)| json!({"date": d.to_string(), "tasks": ts}))
        .collect();
    let v = json!({"overdue": overdue, "days": days_json, "undated_open_tasks": undated});
    if ctx.json {
        out::json(&v);
        return Ok(());
    }
    let print = |title: &str, ts: &[Value]| {
        println!("{title}");
        if ts.is_empty() {
            println!("  (nothing)");
        }
        for t in ts {
            let when = out::short_dt(&t["start"], t["all_day"].as_bool().unwrap_or(false));
            let time = if when.len() > 10 {
                format!("{} ", &when[11..])
            } else {
                String::new()
            };
            println!(
                "  {:>5}  {time}{}  [{}]{}",
                out::s(&t["id"]),
                out::s(&t["title"]),
                out::s(&t["project"]),
                if t["priority"] != "none" {
                    format!(" !{}", out::s(&t["priority"]))
                } else {
                    String::new()
                }
            );
        }
    };
    if !overdue.is_empty() {
        print("Overdue", &overdue);
    }
    for (d, ts) in &by_day {
        let label = if *d == today {
            format!("Today {d}")
        } else {
            format!("{} {d}", d.format("%A"))
        };
        print(&label, ts);
    }
    println!("\n{undated} open task(s) without a date");
    Ok(())
}

/// Dump everything an agent may want as one JSON document.
pub fn export(ctx: &Ctx, include_closed: bool) -> Result<()> {
    let projects: Vec<Value> = model::all_projects(&ctx.conn, &ctx.inbox_id, true)?
        .iter()
        .map(|p| p.to_json(&ctx.conn))
        .collect();
    let mut sql =
        format!("SELECT {TASK_COLS} FROM TaskModel WHERE (deleted = 0 OR deleted IS NULL)");
    if !include_closed {
        sql.push_str(" AND status = 0");
    }
    let mut stmt = ctx.conn.prepare(&sql)?;
    let ts: Vec<Task> = stmt
        .query_map([], Task::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    let tasks: Vec<Value> = ts
        .iter()
        .map(|t| model::task_json(&ctx.conn, &ctx.inbox_id, t, true))
        .collect();
    let tags: Vec<Value> = {
        let mut s = ctx.conn.prepare("SELECT name, label, color, parent FROM TagModel WHERE (deleted IS NULL OR deleted = 0) AND (status IS NULL OR status <> -1)")?;
        s.query_map([], |r| Ok(json!({"name": r.get::<_, Option<String>>(0)?, "label": r.get::<_, Option<String>>(1)?, "color": r.get::<_, Option<String>>(2)?, "parent": r.get::<_, Option<String>>(3)?})))?
            .collect::<rusqlite::Result<_>>()?
    };
    out::json(
        &json!({"exported_at": ctx::now_str(), "user_id": ctx.user_id, "projects": projects, "tags": tags, "tasks": tasks}),
    );
    Ok(())
}

fn row_to_json(r: &rusqlite::Row, names: &[String]) -> rusqlite::Result<Value> {
    let mut m = Map::new();
    for (i, n) in names.iter().enumerate() {
        let v = match r.get_ref(i)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(x) => json!(x),
            ValueRef::Real(x) => json!(x),
            ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
            ValueRef::Blob(b) => json!(format!("<{} bytes>", b.len())),
        };
        m.insert(n.clone(), v);
    }
    Ok(Value::Object(m))
}

/// Run raw SQL. Read-only unless --write; writes go through the same transaction/backup path.
pub fn sql(ctx: &mut Ctx, query: &str, write: bool) -> Result<()> {
    if write {
        let n = ctx.write(|conn, _| Ok(conn.execute_batch(query).map(|_| conn.changes())?))?;
        out::done(
            ctx.json,
            json!({"changes": n}),
            &format!("OK ({n} row(s) changed by the last statement)"),
        );
        return Ok(());
    }
    let mut stmt = ctx.conn.prepare(query)?;
    if !stmt.readonly() {
        bail!("This statement writes; pass --write (the app must be closed)");
    }
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let rows: Vec<Value> = stmt
        .query_map([], |r| row_to_json(r, &names))?
        .collect::<rusqlite::Result<_>>()?;
    if ctx.json {
        out::json(&json!(rows));
    } else {
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let table = rows
            .iter()
            .map(|r| names.iter().map(|n| out::s(&r[n])).collect())
            .collect();
        out::table(&refs, table, "(no rows)");
    }
    Ok(())
}

pub fn schema(ctx: &Ctx, table: Option<&str>) -> Result<()> {
    let mut stmt = ctx.conn.prepare(
        "SELECT name, sql FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND (? IS NULL OR name = ?) ORDER BY name",
    )?;
    let rows: Vec<(String, Option<String>)> = stmt
        .query_map([table, table], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if rows.is_empty() {
        bail!("No such table");
    }
    if ctx.json {
        let v: Vec<Value> = rows
            .iter()
            .map(|(n, _)| {
                let cols: Vec<Value> = ctx
                    .conn
                    .prepare(&format!("PRAGMA table_info(\"{n}\")"))
                    .and_then(|mut s| {
                        s.query_map([], |r| Ok(json!({"name": r.get::<_, String>(1)?, "type": r.get::<_, String>(2)?})))
                            .map(|m| m.filter_map(|x| x.ok()).collect())
                    })
                    .unwrap_or_default();
                json!({"table": n, "columns": cols})
            })
            .collect();
        out::json(&json!(v));
    } else if table.is_some() {
        for (_, s) in rows {
            println!("{}", s.unwrap_or_default());
        }
    } else {
        for (n, _) in rows {
            println!("{n}");
        }
    }
    Ok(())
}

pub fn backup(db: &Path, out_path: Option<&str>, as_json: bool) -> Result<()> {
    let target = match out_path {
        Some(p) => std::path::PathBuf::from(p),
        None => db.with_file_name(format!(
            "{}.cli-backup-{}",
            db.file_name().unwrap_or_default().to_string_lossy(),
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        )),
    };
    if target.exists() {
        bail!("{} already exists", target.display());
    }
    ctx::backup_to(db, &target)?;
    out::done(
        as_json,
        json!({"backup": target.display().to_string()}),
        &format!("Backed up to {}", target.display()),
    );
    Ok(())
}

pub fn comments(ctx: &Ctx, task: &str) -> Result<()> {
    let t = model::resolve_task(&ctx.conn, &ctx.user_id, task)?;
    let mut stmt = ctx.conn.prepare(
        "SELECT title, userName, createdTime FROM CommentModel WHERE taskSid = ? AND (deleted = 0 OR deleted IS NULL) ORDER BY createdTime",
    )?;
    let rows: Vec<Value> = stmt
        .query_map([&t.sid], |r| Ok(json!({"text": r.get::<_, Option<String>>(0)?, "user": r.get::<_, Option<String>>(1)?, "created_time": r.get::<_, Option<String>>(2)?})))?
        .collect::<rusqlite::Result<_>>()?;
    if ctx.json {
        out::json(&json!(rows));
    } else {
        let table = rows
            .iter()
            .map(|r| {
                vec![
                    out::s(&r["created_time"]),
                    out::s(&r["user"]),
                    out::s(&r["text"]),
                ]
            })
            .collect();
        out::table(&["Time", "User", "Comment"], table, "No comments.");
    }
    Ok(())
}

pub fn filters(ctx: &Ctx) -> Result<()> {
    let mut stmt = ctx.conn.prepare(
        "SELECT _Id, id, name, rule FROM FilterModel WHERE (deleted = 0 OR deleted IS NULL) ORDER BY sortOrder",
    )?;
    let rows: Vec<Value> = stmt
        .query_map([], |r| {
            let rule: Option<String> = r.get(3)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "sid": r.get::<_, Option<String>>(1)?, "name": r.get::<_, Option<String>>(2)?,
                "rule": rule.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).unwrap_or(json!(rule)),
            }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    if ctx.json {
        out::json(&json!(rows));
    } else {
        let table = rows
            .iter()
            .map(|r| vec![out::s(&r["id"]), out::s(&r["name"]), r["rule"].to_string()])
            .collect();
        out::table(&["ID", "Filter", "Rule"], table, "No saved filters.");
    }
    Ok(())
}

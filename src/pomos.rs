//! Focus / pomodoro records (PomodoroModel + PomoTask). Listing logic is from upstream.

use crate::ctx::{self, Ctx};
use crate::dates;
use crate::model;
use crate::out;
use anyhow::{Result, bail};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, params, params_from_iter};
use serde_json::{Value, json};

struct Filters {
    where_clause: String,
    params: Vec<SqlValue>,
}

const START: &str = "COALESCE(NULLIF(pt.StartTime, ''), p.StartTime)";
const END: &str = "COALESCE(NULLIF(pt.EndTime, ''), p.EndTime)";
const TASK: &str = "COALESCE(NULLIF(pt.TaskId, ''), NULLIF(p.TaskSid, ''))";
const PROJECT: &str = "COALESCE(NULLIF(pt.ProjectName, ''), pm.name, '')";
const FROM: &str = "FROM PomodoroModel p
     LEFT JOIN PomoTask pt ON pt.PomoId = p.Id
     LEFT JOIN TaskModel tm ON tm.id = COALESCE(NULLIF(pt.TaskId, ''), NULLIF(p.TaskSid, ''))
     LEFT JOIN ProjectModel pm ON pm.id = tm.projectId";

fn filters(
    ctx: &Ctx,
    date: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    task: Option<&str>,
    project: Option<&str>,
) -> Result<Filters> {
    let mut c = vec![
        format!("{START} IS NOT NULL"),
        "(p.SyncStatus IS NULL OR p.SyncStatus <> -1)".to_string(),
    ];
    let mut p: Vec<SqlValue> = Vec::new();
    if let Some(d) = date {
        c.push(format!("date({START}) = ?"));
        p.push(dates::parse_date(d)?.to_string().into());
    }
    if let Some(d) = from {
        c.push(format!("date({START}) >= ?"));
        p.push(dates::parse_date(d)?.to_string().into());
    }
    if let Some(d) = to {
        c.push(format!("date({START}) <= ?"));
        p.push(dates::parse_date(d)?.to_string().into());
    }
    if let Some(t) = task {
        let t = model::resolve_task(&ctx.conn, &ctx.user_id, t)?;
        c.push(format!("{TASK} = ?"));
        p.push(t.sid.into());
    }
    if let Some(pr) = project {
        let pr = model::resolve_project(&ctx.conn, &ctx.inbox_id, pr)?;
        c.push(format!("(tm.projectId = ? OR {PROJECT} = ?)"));
        p.push(pr.sid.into());
        p.push(pr.name.into());
    }
    Ok(Filters {
        where_clause: format!("WHERE {}", c.join(" AND ")),
        params: p,
    })
}

fn secs(start: Option<&str>, end: Option<&str>) -> i64 {
    match (
        start.and_then(dates::parse_db_dt),
        end.and_then(dates::parse_db_dt),
    ) {
        (Some(s), Some(e)) => (e - s).num_seconds().max(0),
        _ => 0,
    }
}

pub fn fmt_duration(seconds: i64) -> String {
    if seconds <= 0 {
        return "0m".into();
    }
    let minutes = (seconds + 30) / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn list(
    ctx: &Ctx,
    date: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    task: Option<&str>,
    project: Option<&str>,
    summary: bool,
    limit: usize,
) -> Result<()> {
    let f = filters(ctx, date, from, to, task, project)?;
    if summary {
        return list_summary(ctx, &f);
    }
    let sql = format!(
        "SELECT COALESCE(pt.PomoId, p.Id), {TASK}, COALESCE(NULLIF(pt.Title, ''), tm.title, ''), {PROJECT},
                {START} AS st, {END}, p.Status, p.Type, COALESCE(p.PauseDuration, 0), COALESCE(p.Note, '')
         {FROM} {} ORDER BY datetime(st) DESC LIMIT ?",
        f.where_clause
    );
    let mut params = f.params.clone();
    params.push(SqlValue::Integer(limit as i64));
    let mut stmt = ctx.conn.prepare(&sql)?;
    let rows: Vec<Value> = stmt
        .query_map(params_from_iter(params.iter()), |r| {
            let st: Option<String> = r.get(4)?;
            let en: Option<String> = r.get(5)?;
            Ok(json!({
                "sid": r.get::<_, Option<String>>(0)?,
                "task_sid": r.get::<_, Option<String>>(1)?,
                "task": r.get::<_, Option<String>>(2)?,
                "project": r.get::<_, Option<String>>(3)?,
                "start": st, "end": en,
                "seconds": secs(st.as_deref(), en.as_deref()),
                "status": match r.get::<_, Option<i64>>(6)? { Some(1) => "completed", Some(0) => "unfinished", _ => "unknown" },
                "type": match r.get::<_, Option<i64>>(7)? { Some(0) => "pomodoro", Some(1) => "stopwatch", _ => "unknown" },
                "note": r.get::<_, Option<String>>(9)?,
            }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    if ctx.json {
        out::json(&json!(rows));
        return Ok(());
    }
    let table = rows
        .iter()
        .map(|r| {
            vec![
                out::s(&r["start"]).chars().take(16).collect(),
                out::s(&r["end"]).chars().skip(11).take(5).collect(),
                fmt_duration(r["seconds"].as_i64().unwrap_or(0)),
                out::s(&r["task"]),
                out::s(&r["project"]),
                out::s(&r["status"]),
                out::s(&r["type"]),
                out::s(&r["note"]),
            ]
        })
        .collect();
    out::table(
        &[
            "Start", "End", "Duration", "Task", "Project", "Status", "Type", "Note",
        ],
        table,
        "No pomodoro records found.",
    );
    Ok(())
}

fn list_summary(ctx: &Ctx, f: &Filters) -> Result<()> {
    let sql = format!(
        "SELECT date({START}) AS day, COUNT(DISTINCT p.Id), COUNT(*),
                SUM(strftime('%s', {END}) - strftime('%s', {START}))
         {FROM} {} GROUP BY day ORDER BY day DESC",
        f.where_clause
    );
    let mut stmt = ctx.conn.prepare(&sql)?;
    let rows: Vec<Value> = stmt
        .query_map(params_from_iter(f.params.iter()), |r| {
            Ok(json!({"date": r.get::<_, String>(0)?, "pomos": r.get::<_, i64>(1)?, "sessions": r.get::<_, i64>(2)?, "seconds": r.get::<_, Option<i64>>(3)?.unwrap_or(0)}))
        })?
        .collect::<rusqlite::Result<_>>()?;
    if ctx.json {
        out::json(&json!(rows));
        return Ok(());
    }
    let table = rows
        .iter()
        .map(|r| {
            vec![
                out::s(&r["date"]),
                out::s(&r["pomos"]),
                out::s(&r["sessions"]),
                fmt_duration(r["seconds"].as_i64().unwrap_or(0)),
            ]
        })
        .collect();
    out::table(
        &["Date", "Pomos", "Sessions", "Duration"],
        table,
        "No pomodoro records found.",
    );
    Ok(())
}

/// Log a finished focus session.
pub fn add(
    ctx: &mut Ctx,
    start: &str,
    end: Option<&str>,
    duration: Option<&str>,
    task: Option<&str>,
    stopwatch: bool,
    note: Option<&str>,
) -> Result<()> {
    let s = dates::parse_when(start)?.dt;
    let e = match (end, duration) {
        (Some(e), _) => {
            let w = dates::parse_when(e)?;
            if w.has_time {
                w.dt
            } else {
                s.date().and_time(w.dt.time())
            }
        }
        (None, Some(d)) => s + dates::parse_duration(d)?,
        (None, None) => s + chrono::Duration::minutes(25),
    };
    if e <= s {
        bail!("End must be after start");
    }
    let t = task
        .map(|k| model::resolve_task(&ctx.conn, &ctx.user_id, k))
        .transpose()?;
    let inbox = ctx.inbox_id.clone();
    let sid = ctx.write(|conn, env| {
        let sid = ctx::new_object_id();
        conn.execute(
            "INSERT INTO PomodoroModel (Id, TaskSid, Status, StartTime, EndTime, AdjustTime, SyncStatus, UserId, Added, Type, PauseDuration, Note)
             VALUES (?, ?, 1, ?, ?, 0, 0, ?, 1, ?, 0, ?)",
            params![sid, t.as_ref().map(|t| t.sid.clone()), ctx::fmt_dt(&s), ctx::fmt_dt(&e), env.user_id, stopwatch as i64, note],
        )?;
        if let Some(t) = &t {
            insert_pomo_task(conn, &sid, t, &inbox, &ctx::fmt_dt(&s), &ctx::fmt_dt(&e))?;
        }
        Ok(sid)
    })?;
    out::done(
        ctx.json,
        json!({"action": "add_pomo", "sid": sid, "start": ctx::fmt_dt(&s), "end": ctx::fmt_dt(&e), "task_sid": t.map(|t| t.sid)}),
        &format!(
            "Logged focus {} - {} ({})",
            ctx::fmt_dt(&s),
            e.format("%H:%M"),
            fmt_duration((e - s).num_seconds())
        ),
    );
    Ok(())
}

fn insert_pomo_task(
    conn: &Connection,
    pomo: &str,
    t: &model::Task,
    inbox: &str,
    s: &str,
    e: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO PomoTask (TaskId, PomoId, Title, ProjectName, TagString, StartTime, EndTime) VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![t.sid, pomo, t.title, model::project_name(conn, inbox, &t.project_id), serde_json::to_string(&t.tags)?, s, e],
    )?;
    Ok(())
}

pub fn delete(ctx: &mut Ctx, sid: &str) -> Result<()> {
    let n = ctx.write(|conn, _| {
        let st: Option<i64> = conn
            .query_row(
                "SELECT SyncStatus FROM PomodoroModel WHERE Id = ?",
                [sid],
                |r| r.get(0),
            )
            .map_err(|_| anyhow::anyhow!("Focus record not found: {sid}"))?;
        if st == Some(0) {
            conn.execute("DELETE FROM PomoTask WHERE PomoId = ?", [sid])?;
            Ok(conn.execute("DELETE FROM PomodoroModel WHERE Id = ?", [sid])?)
        } else {
            Ok(conn.execute(
                "UPDATE PomodoroModel SET SyncStatus = -1 WHERE Id = ?",
                [sid],
            )?)
        }
    })?;
    out::done(
        ctx.json,
        json!({"action": "delete_pomo", "sid": sid, "rows": n}),
        "Deleted focus record",
    );
    Ok(())
}

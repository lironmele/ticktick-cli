//! Habits (HabitModel), daily check-ins (HabitCheckInModel) and check-in notes (HabitRecordModel).
//! Habit.Status: 0 active, 1 archived, -1 deleted. SyncStatus / check-in Status:
//! 0 new, 1 changed, 2 synced, -1 deleted. CheckStatus: 2 done, 1 failed, 0 partial.

use crate::ctx::{self, Ctx, SORT_STEP};
use crate::dates;
use crate::out;
use anyhow::{Result, anyhow, bail};
use chrono::{Duration, NaiveDate};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
struct Habit {
    local_id: i64,
    sid: String,
    name: String,
    kind: String,
    goal: f64,
    step: f64,
    unit: String,
    status: i64,
    sync_status: i64,
    repeat: String,
    color: Option<String>,
    reminder: Option<String>,
    encouragement: Option<String>,
    target_days: i64,
    created: Option<String>,
}

const COLS: &str = "_Id, Id, Name, Type, Goal, Step, Unit, Status, SyncStatus, RepeatRule, Color, Reminder, Encouragement, TargetDays, CreatedTime";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Habit> {
    Ok(Habit {
        local_id: r.get(0)?,
        sid: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
        name: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
        kind: r
            .get::<_, Option<String>>(3)?
            .unwrap_or_else(|| "Boolean".into()),
        goal: r.get::<_, Option<f64>>(4)?.unwrap_or(1.0),
        step: r.get::<_, Option<f64>>(5)?.unwrap_or(1.0),
        unit: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        status: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
        sync_status: r.get::<_, Option<i64>>(8)?.unwrap_or(2),
        repeat: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
        color: r.get(10)?,
        reminder: r.get(11)?,
        encouragement: r.get(12)?,
        target_days: r.get::<_, Option<i64>>(13)?.unwrap_or(0),
        created: r.get::<_, Option<String>>(14)?,
    })
}

impl Habit {
    fn is_bool(&self) -> bool {
        !self.kind.eq_ignore_ascii_case("Real")
    }
}

fn all(conn: &Connection, archived: bool) -> Result<Vec<Habit>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM HabitModel WHERE Status >= 0 AND (SyncStatus IS NULL OR SyncStatus <> -1) ORDER BY Status, SortOrder"
    ))?;
    let mut v: Vec<Habit> = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<_>>()?;
    if !archived {
        v.retain(|h| h.status == 0);
    }
    Ok(v)
}

fn find(conn: &Connection, key: &str) -> Result<Habit> {
    let hs = all(conn, true)?;
    let lower = key.to_lowercase();
    if let Some(h) = hs
        .iter()
        .find(|h| h.sid == key || h.local_id.to_string() == key || h.name.to_lowercase() == lower)
    {
        return Ok(h.clone());
    }
    let hits: Vec<&Habit> = hs
        .iter()
        .filter(|h| h.name.to_lowercase().contains(&lower))
        .collect();
    match hits.len() {
        1 => Ok(hits[0].clone()),
        0 => Err(anyhow!("Habit not found: {key}")),
        _ => Err(anyhow!("Habit '{key}' is ambiguous")),
    }
}

struct CheckIn {
    local_id: i64,
    stamp: String,
    value: f64,
    goal: f64,
    status: i64,
    check_status: i64,
}

fn checkin_for(conn: &Connection, habit: &str, day: NaiveDate) -> Result<Option<CheckIn>> {
    Ok(conn
        .query_row(
            "SELECT _Id, CheckinStamp, Value, Goal, Status, CheckStatus FROM HabitCheckInModel
             WHERE HabitId = ? AND CheckinStamp = ? ORDER BY _Id DESC LIMIT 1",
            params![habit, day.format("%Y%m%d").to_string()],
            |r| {
                Ok(CheckIn {
                    local_id: r.get(0)?,
                    stamp: r.get(1)?,
                    value: r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                    goal: r.get::<_, Option<f64>>(3)?.unwrap_or(1.0),
                    status: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    check_status: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                })
            },
        )
        .optional()?)
}

fn check_name(c: &CheckIn) -> &'static str {
    if c.status == -1 {
        return "none";
    }
    match c.check_status {
        2 => "done",
        1 => "failed",
        _ if c.value > 0.0 => "partial",
        _ => "none",
    }
}

fn streak(conn: &Connection, h: &Habit) -> Result<i64> {
    let mut d = dates::today();
    // today not yet done does not break the streak
    if !checkin_for(conn, &h.sid, d)?.is_some_and(|c| check_name(&c) == "done") {
        d -= Duration::days(1);
    }
    let mut n = 0;
    while let Some(c) = checkin_for(conn, &h.sid, d)? {
        if check_name(&c) != "done" {
            break;
        }
        n += 1;
        d -= Duration::days(1);
    }
    Ok(n)
}

fn habit_json(conn: &Connection, h: &Habit, days: i64) -> Result<Value> {
    let today = dates::today();
    let mut recent = Vec::new();
    for i in (0..days).rev() {
        let d = today - Duration::days(i);
        let c = checkin_for(conn, &h.sid, d)?;
        recent.push(json!({
            "date": d.to_string(),
            "state": c.as_ref().map(check_name).unwrap_or("none"),
            "value": c.as_ref().map(|c| c.value),
        }));
    }
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM HabitCheckInModel WHERE HabitId = ? AND CheckStatus = 2 AND Status <> -1",
        [&h.sid],
        |r| r.get(0),
    )?;
    Ok(json!({
        "id": h.local_id, "sid": h.sid, "name": h.name,
        "type": if h.is_bool() { "boolean" } else { "count" },
        "goal": h.goal, "step": h.step, "unit": h.unit,
        "archived": h.status == 1, "repeat": h.repeat, "color": h.color,
        "reminders": h.reminder, "encouragement": h.encouragement, "target_days": h.target_days,
        "created_time": h.created, "total_checkins": total, "streak": streak(conn, h)?,
        "today": recent.last().map(|r| r["state"].clone()), "recent": recent,
    }))
}

pub fn list(ctx: &Ctx, archived: bool) -> Result<()> {
    let hs = all(&ctx.conn, archived)?;
    let rows: Vec<Value> = hs
        .iter()
        .map(|h| habit_json(&ctx.conn, h, 7))
        .collect::<Result<_>>()?;
    if ctx.json {
        out::json(&json!(rows));
        return Ok(());
    }
    let table = rows
        .iter()
        .map(|r| {
            let week: String = r["recent"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| match d["state"].as_str() {
                    Some("done") => '●',
                    Some("failed") => '✗',
                    Some("partial") => '◐',
                    _ => '·',
                })
                .collect();
            vec![
                out::s(&r["id"]),
                out::s(&r["name"]),
                out::s(&r["today"]),
                week,
                out::s(&r["streak"]),
                out::s(&r["total_checkins"]),
            ]
        })
        .collect();
    out::table(
        &["ID", "Habit", "Today", "Last 7d", "Streak", "Total"],
        table,
        "No habits.",
    );
    Ok(())
}

pub fn show(ctx: &Ctx, key: &str, days: i64) -> Result<()> {
    let h = find(&ctx.conn, key)?;
    let v = habit_json(&ctx.conn, &h, days)?;
    if ctx.json {
        out::json(&v);
        return Ok(());
    }
    for k in [
        "name",
        "type",
        "goal",
        "unit",
        "repeat",
        "archived",
        "streak",
        "total_checkins",
        "encouragement",
    ] {
        println!("{k:<15}{}", out::s(&v[k]));
    }
    println!();
    for d in v["recent"].as_array().unwrap() {
        println!("{}  {}", out::s(&d["date"]), out::s(&d["state"]));
    }
    Ok(())
}

/// Check in (or set a value for count habits), mark failed, or reset a day.
pub fn checkin(
    ctx: &mut Ctx,
    key: &str,
    date: Option<&str>,
    value: Option<f64>,
    set: bool,
    mode: CheckMode,
) -> Result<()> {
    let h = find(&ctx.conn, key)?;
    let day = date
        .map(dates::parse_date)
        .transpose()?
        .unwrap_or_else(dates::today);
    if day > dates::today() {
        bail!("Cannot check in for a future date");
    }
    let state = ctx.write(|conn, env| {
        let existing = checkin_for(conn, &h.sid, day)?;
        let now = ctx::now_str();
        let stamp = day.format("%Y%m%d").to_string();
        match mode {
            CheckMode::Reset => {
                if let Some(c) = existing {
                    if c.status == 0 {
                        conn.execute("DELETE FROM HabitCheckInModel WHERE _Id = ?", [c.local_id])?;
                    } else {
                        conn.execute(
                            "UPDATE HabitCheckInModel SET Value = 0, CheckStatus = 0, Status = -1, opTime = ? WHERE _Id = ?",
                            params![now, c.local_id],
                        )?;
                    }
                }
                return Ok("none");
            }
            CheckMode::Fail => {
                match existing.filter(|c| c.status != -1) {
                    None => {
                        conn.execute(
                            "INSERT INTO HabitCheckInModel (Id, HabitId, CheckinStamp, CheckinTime, opTime, Value, Goal, UserId, Status, CheckStatus)
                             VALUES (?, ?, ?, ?, ?, 0, ?, ?, 0, 1)",
                            params![ctx::new_object_id(), h.sid, stamp, now, now, h.goal, env.user_id],
                        )?;
                    }
                    Some(c) => {
                        conn.execute(
                            "UPDATE HabitCheckInModel SET CheckStatus = 1, Status = ?, opTime = ? WHERE _Id = ?",
                            params![ctx::bump_int_status(Some(c.status)), now, c.local_id],
                        )?;
                    }
                }
                return Ok("failed");
            }
            CheckMode::Done => {}
        }
        let existing = existing.filter(|c| c.status != -1);
        let (new_value, goal) = if h.is_bool() {
            (1.0_f64.max(existing.as_ref().map(|c| c.goal).unwrap_or(1.0)), existing.as_ref().map(|c| c.goal).unwrap_or(1.0))
        } else {
            let base = existing.as_ref().map(|c| c.value.max(0.0)).unwrap_or(0.0);
            let goal = existing.as_ref().map(|c| c.goal).unwrap_or(h.goal);
            let v = match (value, set) {
                (Some(v), true) => v,
                (Some(v), false) => base + v,
                // no value: complete the goal
                (None, _) => goal.max(base),
            };
            (v, goal)
        };
        let done = new_value >= goal;
        let check_time = if done { Some(now.clone()) } else { None };
        match existing {
            None => {
                conn.execute(
                    "INSERT INTO HabitCheckInModel (Id, HabitId, CheckinStamp, CheckinTime, opTime, Value, Goal, UserId, Status, CheckStatus)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?)",
                    params![ctx::new_object_id(), h.sid, stamp, check_time, now, new_value, goal, env.user_id, if done { 2 } else { 0 }],
                )?;
            }
            Some(c) => {
                conn.execute(
                    "UPDATE HabitCheckInModel SET Value = ?, Goal = ?, CheckinTime = ?, CheckStatus = ?, Status = ?, opTime = ? WHERE _Id = ?",
                    params![new_value, goal, check_time, if done { 2 } else { 0 }, ctx::bump_int_status(Some(c.status)), now, c.local_id],
                )?;
                let _ = c.stamp;
            }
        }
        Ok(if done { "done" } else { "partial" })
    })?;
    out::done(
        ctx.json,
        json!({"habit": h.name, "sid": h.sid, "date": day.to_string(), "state": state}),
        &format!("{}: {} on {}", h.name, state, day),
    );
    Ok(())
}

#[derive(Copy, Clone, Debug)]
pub enum CheckMode {
    Done,
    Fail,
    Reset,
}

#[allow(clippy::too_many_arguments)]
pub fn add(
    ctx: &mut Ctx,
    name: &str,
    goal: Option<f64>,
    step: Option<f64>,
    unit: Option<&str>,
    repeat: Option<&str>,
    color: Option<&str>,
    reminders: &[String],
    encouragement: Option<&str>,
) -> Result<()> {
    if all(&ctx.conn, true)?
        .iter()
        .any(|h| h.name.eq_ignore_ascii_case(name))
    {
        bail!("Habit '{name}' already exists");
    }
    let repeat = repeat
        .map(dates::parse_repeat)
        .transpose()?
        .unwrap_or_else(|| "RRULE:FREQ=DAILY;INTERVAL=1".into());
    let is_count = goal.is_some_and(|g| g != 1.0) || step.is_some() || unit.is_some();
    let reminder = reminders
        .iter()
        .map(|r| {
            chrono::NaiveTime::parse_from_str(r, "%H:%M")
                .map(|t| t.format("%H:%M").to_string())
                .map_err(|_| anyhow!("Habit reminders are times like 08:30"))
        })
        .collect::<Result<Vec<_>>>()?
        .join(",");
    let sid = ctx.write(|conn, env| {
        let min: Option<i64> = conn.query_row("SELECT MIN(SortOrder) FROM HabitModel WHERE Status >= 0", [], |r| r.get(0))?;
        let sid = ctx::new_object_id();
        let now = ctx::now_str();
        conn.execute(
            "INSERT INTO HabitModel (Id, Name, UserId, IconRes, Color, Status, TotalCheckIns, CreatedTime, ModifiedTime,
                Type, Goal, Step, Unit, SortOrder, RepeatRule, Encouragement, RecordEnable, TargetDays, TargetStartDate,
                CompletedCycles, Style, Reminder, SyncStatus)
             VALUES (?, ?, ?, 'habit_daily_check_in', ?, 0, 0, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 0, 0, 0, 1, ?, 0)",
            params![
                sid,
                name,
                env.user_id,
                color.unwrap_or("#97E38B"),
                now,
                now,
                if is_count { "Real" } else { "Boolean" },
                goal.unwrap_or(1.0),
                step.unwrap_or(if is_count { 0.0 } else { 1.0 }),
                unit.unwrap_or("Count"),
                min.map(|m| m - SORT_STEP).unwrap_or(0),
                repeat,
                encouragement.unwrap_or(""),
                if reminder.is_empty() { None } else { Some(reminder.clone()) },
            ],
        )?;
        Ok(sid)
    })?;
    out::done(
        ctx.json,
        json!({"action": "add_habit", "sid": sid, "name": name}),
        &format!("Added habit \"{name}\""),
    );
    Ok(())
}

pub fn edit(
    ctx: &mut Ctx,
    key: &str,
    name: Option<&str>,
    goal: Option<f64>,
    unit: Option<&str>,
    repeat: Option<&str>,
    encouragement: Option<&str>,
) -> Result<()> {
    let h = find(&ctx.conn, key)?;
    let repeat = repeat.map(dates::parse_repeat).transpose()?;
    ctx.write(|conn, _| {
        if let Some(n) = name {
            conn.execute("UPDATE HabitModel SET Name = ? WHERE _Id = ?", params![n, h.local_id])?;
        }
        if let Some(g) = goal {
            conn.execute("UPDATE HabitModel SET Goal = ?, Type = CASE WHEN ? = 1.0 AND Type = 'Boolean' THEN 'Boolean' ELSE 'Real' END WHERE _Id = ?", params![g, g, h.local_id])?;
        }
        if let Some(u) = unit {
            conn.execute("UPDATE HabitModel SET Unit = ? WHERE _Id = ?", params![u, h.local_id])?;
        }
        if let Some(r) = &repeat {
            conn.execute("UPDATE HabitModel SET RepeatRule = ? WHERE _Id = ?", params![r, h.local_id])?;
        }
        if let Some(e) = encouragement {
            conn.execute("UPDATE HabitModel SET Encouragement = ? WHERE _Id = ?", params![e, h.local_id])?;
        }
        conn.execute(
            "UPDATE HabitModel SET ModifiedTime = ?, SyncStatus = ? WHERE _Id = ?",
            params![ctx::now_str(), ctx::bump_int_status(Some(h.sync_status)), h.local_id],
        )?;
        Ok(())
    })?;
    out::done(
        ctx.json,
        json!({"action": "edit_habit", "sid": h.sid}),
        &format!("Updated habit \"{}\"", h.name),
    );
    Ok(())
}

pub fn set_archived(ctx: &mut Ctx, key: &str, archived: bool) -> Result<()> {
    let h = find(&ctx.conn, key)?;
    ctx.write(|conn, _| {
        conn.execute(
            "UPDATE HabitModel SET Status = ?, ArchivedTime = ?, ModifiedTime = ?, SyncStatus = ? WHERE _Id = ?",
            params![archived as i64, if archived { Some(ctx::now_str()) } else { None }, ctx::now_str(), ctx::bump_int_status(Some(h.sync_status)), h.local_id],
        )?;
        Ok(())
    })?;
    out::done(
        ctx.json,
        json!({"action": if archived {"archive_habit"} else {"unarchive_habit"}, "sid": h.sid}),
        &format!(
            "{} habit \"{}\"",
            if archived { "Archived" } else { "Restored" },
            h.name
        ),
    );
    Ok(())
}

pub fn delete(ctx: &mut Ctx, key: &str) -> Result<()> {
    let h = find(&ctx.conn, key)?;
    ctx.write(|conn, _| {
        if h.sync_status == 0 {
            conn.execute("DELETE FROM HabitModel WHERE _Id = ?", [h.local_id])?;
        } else {
            conn.execute(
                "UPDATE HabitModel SET Status = -1, SyncStatus = -1, ModifiedTime = ? WHERE _Id = ?",
                params![ctx::now_str(), h.local_id],
            )?;
        }
        conn.execute("DELETE FROM HabitCheckInModel WHERE HabitId = ?", [&h.sid])?;
        conn.execute("DELETE FROM HabitRecordModel WHERE HabitId = ?", [&h.sid])?;
        Ok(())
    })?;
    out::done(
        ctx.json,
        json!({"action": "delete_habit", "sid": h.sid}),
        &format!("Deleted habit \"{}\"", h.name),
    );
    Ok(())
}

/// Attach a note ("log") to a habit day.
pub fn note(ctx: &mut Ctx, key: &str, date: Option<&str>, text: &str) -> Result<()> {
    let h = find(&ctx.conn, key)?;
    let day = date
        .map(dates::parse_date)
        .transpose()?
        .unwrap_or_else(dates::today);
    let stamp: i64 = day.format("%Y%m%d").to_string().parse()?;
    ctx.write(|conn, env| {
        let existing: Option<(i64, Option<i64>)> = conn
            .query_row(
                "SELECT _Id, Status FROM HabitRecordModel WHERE HabitId = ? AND Stamp = ? AND (Deleted = 0 OR Deleted IS NULL)",
                params![h.sid, stamp],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match existing {
            Some((id, st)) => {
                conn.execute(
                    "UPDATE HabitRecordModel SET Content = ?, opTime = ?, Status = ? WHERE _Id = ?",
                    params![text, ctx::now_str(), ctx::bump_int_status(st), id],
                )?;
            }
            None => {
                conn.execute(
                    "INSERT INTO HabitRecordModel (Content, HabitId, Id, Stamp, Emoji, opTime, UserId, Status, Deleted)
                     VALUES (?, ?, ?, ?, 0, ?, ?, 0, 0)",
                    params![text, h.sid, ctx::new_object_id(), stamp, ctx::now_str(), env.user_id],
                )?;
            }
        }
        Ok(())
    })?;
    out::done(
        ctx.json,
        json!({"action": "habit_note", "sid": h.sid, "date": day.to_string()}),
        &format!("Saved note for {} on {day}", h.name),
    );
    Ok(())
}

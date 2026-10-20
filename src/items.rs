//! Checklist items (TaskDetailItemModel). Items sync as part of their task, so
//! every change marks the task modified and refreshes its progress.

use crate::ctx::{self, Ctx, SORT_STEP};
use crate::dates;
use crate::model::{self, Task};
use crate::out;
use crate::tasks;
use anyhow::Result;
use rusqlite::params;
use serde_json::json;

pub fn list(ctx: &Ctx, task: &str) -> Result<()> {
    let t = model::resolve_task(&ctx.conn, &ctx.user_id, task)?;
    let items = model::task_items(&ctx.conn, &t.sid)?;
    if ctx.json {
        out::json(&json!(
            items.iter().map(|i| i.to_json()).collect::<Vec<_>>()
        ));
        return Ok(());
    }
    let rows = items
        .iter()
        .enumerate()
        .map(|(n, i)| {
            vec![
                (n + 1).to_string(),
                if i.status == 1 {
                    "[x]".into()
                } else {
                    "[ ]".into()
                },
                i.title.clone(),
                i.start
                    .clone()
                    .unwrap_or_default()
                    .chars()
                    .take(16)
                    .collect(),
                i.sid.clone(),
            ]
        })
        .collect();
    out::table(
        &["#", "Done", "Title", "Date", "Sid"],
        rows,
        "No checklist items.",
    );
    Ok(())
}

fn reload(ctx: &Ctx, sid: &str) -> Result<Task> {
    Task::load_by_sid(&ctx.conn, sid)?.ok_or_else(|| anyhow::anyhow!("task vanished"))
}

pub fn add(ctx: &mut Ctx, task: &str, titles: &[String], position: Option<usize>) -> Result<()> {
    let t = model::resolve_task(&ctx.conn, &ctx.user_id, task)?;
    ctx.write(|conn, env| {
        let items = model::task_items(conn, &t.sid)?;
        let mut orders: Vec<i64> = items.iter().map(|i| i.sort_order).collect();
        for (n, title) in titles.iter().enumerate() {
            let order = match position {
                // insert before the Nth item (1-based)
                Some(p) if p >= 1 && p <= orders.len() => {
                    let idx = p - 1 + n;
                    let hi = orders.get(idx).copied().unwrap_or(0);
                    let lo = if idx == 0 { hi - SORT_STEP } else { orders[idx - 1] };
                    let o = lo + (hi - lo) / 2;
                    orders.insert(idx, o);
                    o
                }
                _ => {
                    let o = orders.iter().max().map(|m| m + SORT_STEP).unwrap_or(0);
                    orders.push(o);
                    o
                }
            };
            conn.execute(
                "INSERT INTO TaskDetailItemModel (id, TaskId, TaskServerId, title, status, sortOrder)
                 VALUES (?, ?, ?, ?, 0, ?)",
                params![ctx::new_object_id(), t.local_id, t.sid, title, order],
            )?;
        }
        tasks::recompute_progress(conn, env, &t)
    })?;
    let t = reload(ctx, &t.sid)?;
    out::done(
        ctx.json,
        model::task_json(&ctx.conn, &ctx.inbox_id, &t, true),
        &format!("Added {} item(s) to \"{}\"", titles.len(), t.title),
    );
    Ok(())
}

pub fn set_done(
    ctx: &mut Ctx,
    task: &str,
    keys: &[String],
    done: bool,
    complete_task: bool,
) -> Result<()> {
    let t = model::resolve_task(&ctx.conn, &ctx.user_id, task)?;
    let items: Vec<model::Item> = keys
        .iter()
        .map(|k| model::resolve_item(&ctx.conn, k, Some(&t.sid)))
        .collect::<Result<_>>()?;
    ctx.write(|conn, env| {
        for i in &items {
            conn.execute(
                "UPDATE TaskDetailItemModel SET status = ?, completedTime = ? WHERE _Id = ?",
                params![
                    done as i64,
                    if done { Some(ctx::now_str()) } else { None },
                    i.local_id
                ],
            )?;
        }
        tasks::recompute_progress(conn, env, &t)?;
        let t2 = Task::load_by_sid(conn, &t.sid)?.unwrap();
        if done && complete_task && t2.progress >= 100 && t2.status == 0 {
            tasks::complete_for_items(conn, env, &t2)?;
        }
        Ok(())
    })?;
    let t = reload(ctx, &t.sid)?;
    out::done(
        ctx.json,
        model::task_json(&ctx.conn, &ctx.inbox_id, &t, true),
        &format!(
            "{} {} item(s); \"{}\" is {}% done",
            if done { "Checked" } else { "Unchecked" },
            items.len(),
            t.title,
            t.progress
        ),
    );
    Ok(())
}

pub fn edit(
    ctx: &mut Ctx,
    task: &str,
    key: &str,
    title: Option<&str>,
    date: Option<&str>,
    clear_date: bool,
) -> Result<()> {
    let t = model::resolve_task(&ctx.conn, &ctx.user_id, task)?;
    let item = model::resolve_item(&ctx.conn, key, Some(&t.sid))?;
    let when = date.map(dates::parse_when).transpose()?;
    ctx.write(|conn, env| {
        if let Some(tt) = title {
            conn.execute("UPDATE TaskDetailItemModel SET title = ? WHERE _Id = ?", params![tt, item.local_id])?;
        }
        if let Some(w) = when {
            conn.execute(
                "UPDATE TaskDetailItemModel SET startDate = ?, isAllDay = ? WHERE _Id = ?",
                params![ctx::fmt_dt(&w.dt), (!w.has_time) as i64, item.local_id],
            )?;
        }
        if clear_date {
            conn.execute(
                "UPDATE TaskDetailItemModel SET startDate = NULL, isAllDay = NULL, snoozeReminderTime = NULL WHERE _Id = ?",
                [item.local_id],
            )?;
        }
        tasks::recompute_progress(conn, env, &t)
    })?;
    let t = reload(ctx, &t.sid)?;
    out::done(
        ctx.json,
        model::task_json(&ctx.conn, &ctx.inbox_id, &t, true),
        "Updated checklist item",
    );
    Ok(())
}

pub fn delete(ctx: &mut Ctx, task: &str, keys: &[String]) -> Result<()> {
    let t = model::resolve_task(&ctx.conn, &ctx.user_id, task)?;
    let items: Vec<model::Item> = keys
        .iter()
        .map(|k| model::resolve_item(&ctx.conn, k, Some(&t.sid)))
        .collect::<Result<_>>()?;
    ctx.write(|conn, env| {
        for i in &items {
            conn.execute(
                "DELETE FROM TaskDetailItemModel WHERE _Id = ?",
                [i.local_id],
            )?;
        }
        tasks::recompute_progress(conn, env, &t)
    })?;
    let t = reload(ctx, &t.sid)?;
    out::done(
        ctx.json,
        model::task_json(&ctx.conn, &ctx.inbox_id, &t, true),
        &format!("Deleted {} item(s)", items.len()),
    );
    Ok(())
}

/// Turn a checklist item into a subtask.
pub fn promote(ctx: &mut Ctx, task: &str, key: &str) -> Result<()> {
    let t = model::resolve_task(&ctx.conn, &ctx.user_id, task)?;
    let item = model::resolve_item(&ctx.conn, key, Some(&t.sid))?;
    // Reuse `tasks add` for the subtask so it gets every default.
    let args = tasks::AddArgs {
        title: item.title.clone(),
        project: None,
        content: None,
        priority: None,
        tag: vec![],
        date: item.start.clone(),
        end: None,
        duration: None,
        all_day: item.start.as_ref().map(|_| item.all_day),
        reminder: vec![],
        repeat: None,
        repeat_from: None,
        parent: Some(t.sid.clone()),
        item: vec![],
        note: false,
        column: None,
        pin: false,
        top: false,
        bottom: true,
        completed: item.status == 1,
    };
    let sid = tasks::add_inner(ctx, &args)?;
    ctx.write(|conn, env| {
        conn.execute(
            "DELETE FROM TaskDetailItemModel WHERE _Id = ?",
            [item.local_id],
        )?;
        tasks::recompute_progress(conn, env, &t)
    })?;
    let sub = reload(ctx, &sid)?;
    out::done(
        ctx.json,
        model::task_json(&ctx.conn, &ctx.inbox_id, &sub, true),
        &format!(
            "Made \"{}\" subtask {} of \"{}\"",
            item.title, sub.local_id, t.title
        ),
    );
    Ok(())
}

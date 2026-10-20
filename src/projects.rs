//! Projects (lists), folders (ProjectGroupModel) and kanban sections (ColumnModel).
//! Projects and folders track sync with sync_status SYNC_NEW / SYNC_UPDATE / SYNC_DONE;
//! columns use "new" / "update" / "done" / "delete".

use crate::ctx::{self, Ctx, SORT_STEP, sync};
use crate::model::{self, Project};
use crate::out;
use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

const SORT_OPTION: &str = r#"{"groupBy":"sortOrder","orderBy":"sortOrder"}"#;

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ViewMode {
    List,
    Kanban,
    Timeline,
}

impl ViewMode {
    fn as_str(self) -> &'static str {
        match self {
            ViewMode::List => "list",
            ViewMode::Kanban => "kanban",
            ViewMode::Timeline => "timeline",
        }
    }
}

pub fn list(ctx: &Ctx, include_archived: bool) -> Result<()> {
    let projects = model::all_projects(&ctx.conn, &ctx.inbox_id, include_archived)?;
    let rows: Vec<Value> = projects.iter().map(|p| p.to_json(&ctx.conn)).collect();
    if ctx.json {
        out::json(&json!(rows));
        return Ok(());
    }
    let table = rows
        .iter()
        .map(|r| {
            let mut name = out::s(&r["name"]);
            if r["archived"].as_bool() == Some(true) {
                name.push_str(" (archived)");
            }
            vec![
                out::s(&r["id"]),
                name,
                out::s(&r["folder"]),
                out::s(&r["kind"]),
                out::s(&r["view_mode"]),
                out::s(&r["open_tasks"]),
            ]
        })
        .collect();
    out::table(
        &["ID", "Name", "Folder", "Kind", "View", "Open"],
        table,
        "No projects found.",
    );
    Ok(())
}

pub fn show(ctx: &Ctx, key: &str) -> Result<()> {
    let p = model::resolve_project(&ctx.conn, &ctx.inbox_id, key)?;
    let mut v = p.to_json(&ctx.conn);
    v["sections"] = json!(columns_of(&ctx.conn, &p.sid)?);
    if ctx.json {
        out::json(&v);
    } else {
        for (k, val) in v.as_object().unwrap() {
            println!("{k:<12} {}", out::s(val));
        }
    }
    Ok(())
}

fn new_project_order(conn: &Connection) -> Result<i64> {
    let min: Option<i64> = conn.query_row(
        "SELECT MIN(o) FROM (
            SELECT sortOrder o FROM ProjectModel WHERE (delete_status IS NULL OR delete_status = 0) AND (Isinbox IS NULL OR Isinbox = 0) AND (teamId IS NULL OR teamId = '')
            UNION ALL SELECT sortOrder FROM ProjectGroupModel WHERE (deleted IS NULL OR deleted = 0) AND (teamId IS NULL OR teamId = ''))",
        [],
        |r| r.get(0),
    )?;
    Ok(min.map(|m| m - SORT_STEP).unwrap_or(0))
}

fn resolve_folder(conn: &Connection, key: &str) -> Result<(String, String)> {
    let mut stmt = conn.prepare(
        "SELECT id, name, _Id FROM ProjectGroupModel WHERE (deleted IS NULL OR deleted = 0)",
    )?;
    let all: Vec<(String, String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let lower = key.to_lowercase();
    all.into_iter()
        .find(|(id, name, l)| id == key || l.to_string() == key || name.to_lowercase() == lower)
        .map(|(id, name, _)| (id, name))
        .ok_or_else(|| anyhow!("Folder not found: {key}"))
}

fn mark_project_changed(conn: &Connection, sid: &str) -> Result<()> {
    conn.execute(
        "UPDATE ProjectModel SET sync_status = 'SYNC_UPDATE' WHERE id = ? AND (sync_status IS NULL OR sync_status IN ('SYNC_DONE', 'SYNC_INIT', ''))",
        [sid],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn add(
    ctx: &mut Ctx,
    name: &str,
    color: Option<&str>,
    folder: Option<&str>,
    note: bool,
    view: Option<ViewMode>,
    muted: bool,
    hide_from_all: bool,
) -> Result<()> {
    if name.chars().any(|c| "\\/'\":*?<>|".contains(c)) {
        bail!("Project names cannot contain \\ / ' \" : * ? < > |");
    }
    if model::all_projects(&ctx.conn, &ctx.inbox_id, true)?
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case(name))
    {
        bail!("A project named '{name}' already exists");
    }
    let folder = folder.map(|f| resolve_folder(&ctx.conn, f)).transpose()?;
    let sid = ctx.write(|conn, env| {
        let sid = ctx::new_object_id();
        let order = match &folder {
            Some((gid, _)) => {
                let min: Option<i64> = conn.query_row(
                    "SELECT MIN(sortOrder) FROM ProjectModel WHERE groupId = ? AND (delete_status IS NULL OR delete_status = 0)",
                    [gid],
                    |r| r.get(0),
                )?;
                min.map(|m| m - SORT_STEP).unwrap_or(0)
            }
            None => new_project_order(conn)?,
        };
        conn.execute(
            "INSERT INTO ProjectModel (id, userid, name, Isinbox, Ishide, muted, sortType, transferred, needPullRemote,
                SortOptionString, color, sync_status, delete_status, isOwner, inAll, openToTeam, needAudit, sortOrder,
                userCount, closed, groupId, viewMode, kind, ShowAddColumn, modifiedTime)
             VALUES (?, ?, ?, 0, 0, ?, 'sortOrder', 0, 0, ?, ?, 'SYNC_NEW', 0, 1, ?, 0, 0, ?, 1, 0, ?, ?, ?, 1, ?)",
            params![
                sid,
                env.user_id,
                name,
                muted as i64,
                SORT_OPTION,
                color,
                (!hide_from_all) as i64,
                order,
                folder.as_ref().map(|f| f.0.clone()),
                view.unwrap_or(ViewMode::List).as_str(),
                if note { "NOTE" } else { "TASK" },
                ctx::now_str(),
            ],
        )?;
        if matches!(view, Some(ViewMode::Kanban)) {
            insert_column(conn, &env.user_id, &sid, "Not Sectioned", 0)?;
        }
        Ok(sid)
    })?;
    let p = model::resolve_project(&ctx.conn, &ctx.inbox_id, &sid)
        .unwrap_or_else(|_| Project::inbox(&sid));
    out::done(
        ctx.json,
        p.to_json(&ctx.conn),
        &format!("Added project \"{name}\" (sid {sid})"),
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn edit(
    ctx: &mut Ctx,
    key: &str,
    name: Option<&str>,
    color: Option<&str>,
    folder: Option<&str>,
    no_folder: bool,
    view: Option<ViewMode>,
    muted: Option<bool>,
    show_in_all: Option<bool>,
    kind: Option<&str>,
) -> Result<()> {
    let p = model::resolve_project(&ctx.conn, &ctx.inbox_id, key)?;
    if p.local_id.is_none() {
        bail!("The inbox has no stored settings to edit");
    }
    let folder = folder.map(|f| resolve_folder(&ctx.conn, f)).transpose()?;
    let kind = kind
        .map(|k| match k.to_ascii_lowercase().as_str() {
            "task" => Ok("TASK"),
            "note" => Ok("NOTE"),
            o => Err(anyhow!("unknown project kind '{o}'")),
        })
        .transpose()?;
    ctx.write(|conn, env| {
        let sid = &p.sid;
        if let Some(n) = name {
            conn.execute(
                "UPDATE ProjectModel SET name = ? WHERE id = ?",
                params![n, sid],
            )?;
        }
        if let Some(c) = color {
            let c = if c.is_empty() || c == "none" {
                None
            } else {
                Some(c)
            };
            conn.execute(
                "UPDATE ProjectModel SET color = ? WHERE id = ?",
                params![c, sid],
            )?;
        }
        if let Some((gid, _)) = &folder {
            conn.execute(
                "UPDATE ProjectModel SET groupId = ? WHERE id = ?",
                params![gid, sid],
            )?;
        }
        if no_folder {
            conn.execute("UPDATE ProjectModel SET groupId = NULL WHERE id = ?", [sid])?;
        }
        if let Some(v) = view {
            conn.execute(
                "UPDATE ProjectModel SET viewMode = ? WHERE id = ?",
                params![v.as_str(), sid],
            )?;
            if matches!(v, ViewMode::Kanban) && columns_of(conn, sid)?.is_empty() {
                insert_column(conn, &env.user_id, sid, "Not Sectioned", 0)?;
            }
        }
        if let Some(m) = muted {
            conn.execute(
                "UPDATE ProjectModel SET muted = ? WHERE id = ?",
                params![m as i64, sid],
            )?;
        }
        if let Some(s) = show_in_all {
            conn.execute(
                "UPDATE ProjectModel SET inAll = ? WHERE id = ?",
                params![s as i64, sid],
            )?;
        }
        if let Some(k) = kind {
            conn.execute(
                "UPDATE ProjectModel SET kind = ? WHERE id = ?",
                params![k, sid],
            )?;
        }
        conn.execute(
            "UPDATE ProjectModel SET modifiedTime = ? WHERE id = ?",
            params![ctx::now_str(), sid],
        )?;
        mark_project_changed(conn, sid)
    })?;
    let p = model::resolve_project(&ctx.conn, &ctx.inbox_id, &p.sid)?;
    out::done(
        ctx.json,
        p.to_json(&ctx.conn),
        &format!("Updated project \"{}\"", p.name),
    );
    Ok(())
}

pub fn set_archived(ctx: &mut Ctx, key: &str, archived: bool) -> Result<()> {
    let p = model::resolve_project(&ctx.conn, &ctx.inbox_id, key)?;
    if p.inbox {
        bail!("The inbox cannot be archived");
    }
    ctx.write(|conn, _| {
        conn.execute(
            "UPDATE ProjectModel SET closed = ?, modifiedTime = ? WHERE id = ?",
            params![archived as i64, ctx::now_str(), p.sid],
        )?;
        mark_project_changed(conn, &p.sid)
    })?;
    out::done(
        ctx.json,
        json!({"action": if archived {"archive"} else {"unarchive"}, "sid": p.sid, "name": p.name}),
        &format!(
            "{} project \"{}\"",
            if archived { "Archived" } else { "Unarchived" },
            p.name
        ),
    );
    Ok(())
}

pub fn delete(ctx: &mut Ctx, key: &str, move_tasks_to: Option<&str>) -> Result<()> {
    let p = model::resolve_project(&ctx.conn, &ctx.inbox_id, key)?;
    if p.inbox {
        bail!("The inbox cannot be deleted");
    }
    let target = move_tasks_to
        .map(|k| model::resolve_project(&ctx.conn, &ctx.inbox_id, k))
        .transpose()?;
    let n = ctx.write(|conn, env| {
        let tasks: Vec<String> = {
            let mut stmt = conn.prepare("SELECT id FROM TaskModel WHERE projectId = ? AND (deleted = 0 OR deleted IS NULL)")?;
            stmt.query_map([&p.sid], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        for t in &tasks {
            match &target {
                Some(tp) => {
                    conn.execute(
                        "UPDATE TaskModel SET projectId = ?, columnId = NULL, modifiedTime = ? WHERE id = ?",
                        params![tp.sid, ctx::now_str(), t],
                    )?;
                    ctx::add_move_status(conn, env, t, &p.sid, sync::MOVE)?;
                }
                None => {
                    conn.execute(
                        "UPDATE TaskModel SET deleted = 1, modifiedTime = ? WHERE id = ?",
                        params![ctx::now_str(), t],
                    )?;
                    ctx::add_sync_status(conn, env, t, sync::TRASH)?;
                }
            }
        }
        if p.sync_status.as_deref() == Some("SYNC_NEW") {
            conn.execute("DELETE FROM ProjectModel WHERE id = ?", [&p.sid])?;
        } else {
            conn.execute(
                "UPDATE ProjectModel SET delete_status = 1, sync_status = 'SYNC_UPDATE', modifiedTime = ? WHERE id = ?",
                params![ctx::now_str(), p.sid],
            )?;
        }
        Ok(tasks.len())
    })?;
    let what = if target.is_some() { "moved" } else { "trashed" };
    out::done(
        ctx.json,
        json!({"action": "delete_project", "sid": p.sid, "name": p.name, "tasks": n, "tasks_action": what}),
        &format!("Deleted project \"{}\" ({n} task(s) {what})", p.name),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Folders
// ---------------------------------------------------------------------------

pub fn folders_list(ctx: &Ctx) -> Result<()> {
    let mut stmt = ctx.conn.prepare(
        "SELECT _Id, id, name, sortOrder FROM ProjectGroupModel WHERE (deleted IS NULL OR deleted = 0) ORDER BY sortOrder",
    )?;
    let rows: Vec<Value> = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<i64>>(3)?))
        })?
        .filter_map(|r| r.ok())
        .map(|(l, sid, name, order)| {
            let projects: Vec<String> = ctx
                .conn
                .prepare("SELECT name FROM ProjectModel WHERE groupId = ? AND (delete_status IS NULL OR delete_status = 0) ORDER BY sortOrder")
                .and_then(|mut s| s.query_map([&sid], |r| r.get(0)).map(|m| m.filter_map(|x| x.ok()).collect()))
                .unwrap_or_default();
            json!({"id": l, "sid": sid, "name": name, "sort_order": order, "projects": projects})
        })
        .collect();
    if ctx.json {
        out::json(&json!(rows));
        return Ok(());
    }
    let table = rows
        .iter()
        .map(|r| vec![out::s(&r["id"]), out::s(&r["name"]), out::s(&r["projects"])])
        .collect();
    out::table(&["ID", "Folder", "Projects"], table, "No folders.");
    Ok(())
}

pub fn folders_add(ctx: &mut Ctx, name: &str) -> Result<()> {
    if resolve_folder(&ctx.conn, name).is_ok() {
        bail!("Folder '{name}' already exists");
    }
    let sid = ctx.write(|conn, env| {
        let sid = ctx::new_object_id();
        let order = new_project_order(conn)?;
        conn.execute(
            "INSERT INTO ProjectGroupModel (id, name, sortOrder, deleted, userId, sortType, open, showAll, sync_status,
                SortOptionString, viewMode)
             VALUES (?, ?, ?, 0, ?, 'project', 1, 1, 'SYNC_NEW', ?, 'list')",
            params![sid, name, order, env.user_id.parse::<i64>().unwrap_or(0), SORT_OPTION],
        )?;
        Ok(sid)
    })?;
    out::done(
        ctx.json,
        json!({"action": "add_folder", "sid": sid, "name": name}),
        &format!("Added folder \"{name}\""),
    );
    Ok(())
}

pub fn folders_rename(ctx: &mut Ctx, key: &str, name: &str) -> Result<()> {
    let (sid, old) = resolve_folder(&ctx.conn, key)?;
    ctx.write(|conn, _| {
        conn.execute(
            "UPDATE ProjectGroupModel SET name = ?, sync_status = CASE WHEN sync_status = 'SYNC_NEW' THEN sync_status ELSE 'SYNC_UPDATE' END WHERE id = ?",
            params![name, sid],
        )?;
        Ok(())
    })?;
    out::done(
        ctx.json,
        json!({"action": "rename_folder", "sid": sid, "name": name}),
        &format!("Renamed folder \"{old}\" -> \"{name}\""),
    );
    Ok(())
}

pub fn folders_delete(ctx: &mut Ctx, key: &str) -> Result<()> {
    let (sid, name) = resolve_folder(&ctx.conn, key)?;
    ctx.write(|conn, _| {
        let projects: Vec<String> = {
            let mut s = conn.prepare("SELECT id FROM ProjectModel WHERE groupId = ?")?;
            s.query_map([&sid], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        for p in &projects {
            conn.execute("UPDATE ProjectModel SET groupId = NULL WHERE id = ?", [p])?;
            mark_project_changed(conn, p)?;
        }
        let is_new: bool = conn
            .query_row("SELECT sync_status = 'SYNC_NEW' FROM ProjectGroupModel WHERE id = ?", [&sid], |r| r.get(0))
            .optional()?
            .unwrap_or(false);
        if is_new {
            conn.execute("DELETE FROM ProjectGroupModel WHERE id = ?", [&sid])?;
        } else {
            conn.execute("UPDATE ProjectGroupModel SET deleted = 1, sync_status = 'SYNC_UPDATE' WHERE id = ?", [&sid])?;
        }
        Ok(())
    })?;
    out::done(
        ctx.json,
        json!({"action": "delete_folder", "sid": sid, "name": name}),
        &format!("Deleted folder \"{name}\" (its projects were kept)"),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Kanban sections
// ---------------------------------------------------------------------------

fn columns_of(conn: &Connection, project: &str) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT _Id, id, name, sortOrder FROM ColumnModel WHERE projectId = ? AND (deleted IS NULL OR deleted = 0) ORDER BY sortOrder",
    )?;
    Ok(stmt
        .query_map([project], |r| {
            Ok(json!({"id": r.get::<_, i64>(0)?, "sid": r.get::<_, String>(1)?, "name": r.get::<_, Option<String>>(2)?, "sort_order": r.get::<_, Option<i64>>(3)?}))
        })?
        .collect::<rusqlite::Result<_>>()?)
}

fn insert_column(
    conn: &Connection,
    user: &str,
    project: &str,
    name: &str,
    order: i64,
) -> Result<String> {
    let sid = ctx::new_object_id();
    let now = ctx::now_str();
    conn.execute(
        "INSERT INTO ColumnModel (id, userId, projectId, name, sortOrder, deleted, createdTime, modifiedTime, syncStatus)
         VALUES (?, ?, ?, ?, ?, 0, ?, ?, 'new')",
        params![sid, user, project, name, order, now, now],
    )?;
    Ok(sid)
}

pub fn columns_list(ctx: &Ctx, project: &str) -> Result<()> {
    let p = model::resolve_project(&ctx.conn, &ctx.inbox_id, project)?;
    let mut cols = columns_of(&ctx.conn, &p.sid)?;
    for c in cols.iter_mut() {
        let n: i64 = ctx.conn.query_row(
            "SELECT COUNT(*) FROM TaskModel WHERE columnId = ? AND status = 0 AND (deleted = 0 OR deleted IS NULL)",
            [out::s(&c["sid"])],
            |r| r.get(0),
        )?;
        c["open_tasks"] = json!(n);
    }
    if ctx.json {
        out::json(&json!(cols));
        return Ok(());
    }
    let rows = cols
        .iter()
        .map(|c| {
            vec![
                out::s(&c["id"]),
                out::s(&c["name"]),
                out::s(&c["open_tasks"]),
            ]
        })
        .collect();
    out::table(
        &["ID", "Section", "Open"],
        rows,
        "No sections (project is not in kanban view?).",
    );
    Ok(())
}

pub fn columns_add(ctx: &mut Ctx, project: &str, name: &str) -> Result<()> {
    let p = model::resolve_project(&ctx.conn, &ctx.inbox_id, project)?;
    let sid = ctx.write(|conn, env| {
        let max: Option<i64> = conn.query_row(
            "SELECT MAX(sortOrder) FROM ColumnModel WHERE projectId = ? AND (deleted IS NULL OR deleted = 0)",
            [&p.sid],
            |r| r.get(0),
        )?;
        insert_column(conn, &env.user_id, &p.sid, name, max.map(|m| m + SORT_STEP).unwrap_or(0))
    })?;
    out::done(
        ctx.json,
        json!({"action": "add_section", "sid": sid, "name": name, "project": p.name}),
        &format!("Added section \"{name}\" to \"{}\"", p.name),
    );
    Ok(())
}

pub fn columns_rename(ctx: &mut Ctx, project: &str, key: &str, name: &str) -> Result<()> {
    let p = model::resolve_project(&ctx.conn, &ctx.inbox_id, project)?;
    let sid = crate::tasks::resolve_column(&ctx.conn, &p.sid, key)?;
    ctx.write(|conn, _| {
        conn.execute(
            "UPDATE ColumnModel SET name = ?, modifiedTime = ?, syncStatus = CASE WHEN syncStatus = 'new' THEN 'new' ELSE 'update' END WHERE id = ?",
            params![name, ctx::now_str(), sid],
        )?;
        Ok(())
    })?;
    out::done(
        ctx.json,
        json!({"action": "rename_section", "sid": sid, "name": name}),
        &format!("Renamed section to \"{name}\""),
    );
    Ok(())
}

pub fn columns_delete(
    ctx: &mut Ctx,
    project: &str,
    key: &str,
    move_to: Option<&str>,
) -> Result<()> {
    let p = model::resolve_project(&ctx.conn, &ctx.inbox_id, project)?;
    let sid = crate::tasks::resolve_column(&ctx.conn, &p.sid, key)?;
    let target = match move_to {
        Some(m) => Some(crate::tasks::resolve_column(&ctx.conn, &p.sid, m)?),
        None => columns_of(&ctx.conn, &p.sid)?
            .into_iter()
            .map(|c| out::s(&c["sid"]))
            .find(|c| *c != sid),
    };
    let Some(target) = target else {
        bail!("Cannot delete the only section of a kanban project")
    };
    let n = ctx.write(|conn, env| {
        let tasks: Vec<String> = {
            let mut s = conn.prepare("SELECT id FROM TaskModel WHERE columnId = ?")?;
            s.query_map([&sid], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        for t in &tasks {
            conn.execute("UPDATE TaskModel SET columnId = ?, modifiedTime = ? WHERE id = ?", params![target, ctx::now_str(), t])?;
            ctx::add_sync_status(conn, env, t, sync::MODIFY)?;
        }
        conn.execute(
            "UPDATE ColumnModel SET deleted = 1, modifiedTime = ?, syncStatus = 'delete' WHERE id = ?",
            params![ctx::now_str(), sid],
        )?;
        Ok(tasks.len())
    })?;
    out::done(
        ctx.json,
        json!({"action": "delete_section", "sid": sid, "moved_tasks": n}),
        &format!("Deleted section ({n} task(s) moved)"),
    );
    Ok(())
}

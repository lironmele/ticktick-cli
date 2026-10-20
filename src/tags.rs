//! Tags (TagModel). Tasks store lowercased tag names as a JSON array in TaskModel.tag.
//! TagModel.status: 0 = new, 1 = changed, 2 = synced, -1 = deleted.

use crate::ctx::{self, Ctx, SORT_STEP, WriteEnv, sync};
use crate::model;
use crate::out;
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, params};
use serde_json::json;

const PALETTE: [&str; 10] = [
    "#FF6161", "#FFAC38", "#FFD324", "#E6EA49", "#35D870", "#4CA1FF", "#6E75F4", "#A56FFF",
    "#FF8FC8", "#5BCBBD",
];

#[derive(Debug, Clone)]
struct Tag {
    local_id: i64,
    sid: Option<String>,
    name: String,
    label: String,
    color: Option<String>,
    parent: Option<String>,
    status: Option<i64>,
}

fn live_tags(conn: &Connection) -> Result<Vec<Tag>> {
    let mut stmt = conn.prepare(
        "SELECT _Id, id, name, label, color, parent, sortOrder, status FROM TagModel
         WHERE (deleted = 0 OR deleted IS NULL) AND (status IS NULL OR status <> -1)
         ORDER BY sortOrder",
    )?;
    Ok(stmt
        .query_map([], |r| {
            let name: String = r.get::<_, Option<String>>(2)?.unwrap_or_default();
            Ok(Tag {
                local_id: r.get(0)?,
                sid: r.get(1)?,
                label: r
                    .get::<_, Option<String>>(3)?
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| name.clone()),
                name,
                color: r.get(4)?,
                parent: r.get::<_, Option<String>>(5)?.filter(|p| !p.is_empty()),
                status: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

fn find(conn: &Connection, key: &str) -> Result<Tag> {
    let k = key.trim_start_matches('#').to_lowercase();
    live_tags(conn)?
        .into_iter()
        .find(|t| t.name == k || t.sid.as_deref() == Some(key) || t.local_id.to_string() == key)
        .ok_or_else(|| anyhow!("Tag not found: {key}"))
}

fn task_counts(conn: &Connection) -> Result<std::collections::HashMap<String, i64>> {
    let mut stmt = conn.prepare(
        "SELECT tag FROM TaskModel WHERE tag IS NOT NULL AND tag <> '' AND status = 0 AND (deleted = 0 OR deleted IS NULL)",
    )?;
    let mut m = std::collections::HashMap::new();
    for r in stmt.query_map([], |r| r.get::<_, String>(0))? {
        for t in model::parse_json_list(Some(&r?)) {
            *m.entry(t.to_lowercase()).or_insert(0) += 1;
        }
    }
    Ok(m)
}

pub fn list(ctx: &Ctx) -> Result<()> {
    let tags = live_tags(&ctx.conn)?;
    let counts = task_counts(&ctx.conn)?;
    let rows: Vec<serde_json::Value> = tags
        .iter()
        .map(|t| {
            json!({
                "id": t.local_id, "sid": t.sid, "name": t.name, "label": t.label,
                "color": t.color, "parent": t.parent, "open_tasks": counts.get(&t.name).copied().unwrap_or(0),
            })
        })
        .collect();
    if ctx.json {
        out::json(&json!(rows));
        return Ok(());
    }
    let table = rows
        .iter()
        .map(|r| {
            vec![
                out::s(&r["label"]),
                out::s(&r["parent"]),
                out::s(&r["color"]),
                out::s(&r["open_tasks"]),
            ]
        })
        .collect();
    out::table(
        &["Name", "Parent", "Color", "Open tasks"],
        table,
        "No tags found.",
    );
    Ok(())
}

pub fn insert_tag(
    conn: &Connection,
    env: &WriteEnv,
    label: &str,
    color: Option<&str>,
    parent: Option<&str>,
) -> Result<()> {
    let name = label.to_lowercase();
    let max: Option<i64> = conn.query_row(
        "SELECT MAX(sortOrder) FROM TagModel WHERE (deleted = 0 OR deleted IS NULL)",
        [],
        |r| r.get(0),
    )?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM TagModel", [], |r| r.get(0))?;
    let color = color
        .map(str::to_string)
        .unwrap_or_else(|| PALETTE[(count as usize) % PALETTE.len()].to_string());
    // A previously deleted tag with the same name is revived rather than duplicated.
    let revived = conn.execute(
        "UPDATE TagModel SET deleted = 0, status = 1, label = ?, color = ?, parent = ? WHERE name = ?",
        params![label, color, parent, name],
    )?;
    if revived > 0 {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO TagModel (id, name, label, sortOrder, sortType, status, userId, type, SortOptionString,
            pinyin, inits, deleted, color, parent)
         VALUES (?, ?, ?, ?, 'project', 0, ?, 1, ?, ?, ?, 0, ?, ?)",
        params![
            ctx::new_object_id(),
            name,
            label,
            max.map(|m| m + SORT_STEP).unwrap_or(0),
            env.user_id,
            r#"{"groupBy":"project","orderBy":"sortOrder"}"#,
            name,
            name.chars().next().map(|c| c.to_string()).unwrap_or_default(),
            color,
            parent,
        ],
    )?;
    Ok(())
}

pub fn add(ctx: &mut Ctx, label: &str, color: Option<&str>, parent: Option<&str>) -> Result<()> {
    let label = label.trim_start_matches('#').trim().to_string();
    if find(&ctx.conn, &label).is_ok() {
        bail!("Tag '{label}' already exists");
    }
    let parent = parent
        .map(|p| find(&ctx.conn, p).map(|t| t.name))
        .transpose()?;
    ctx.write(|conn, env| {
        if label
            .chars()
            .any(|c| "\\/\"'*?<>|:,;".contains(c) || c.is_whitespace())
        {
            bail!("Invalid tag name '{label}'");
        }
        insert_tag(conn, env, &label, color, parent.as_deref())
    })?;
    out::done(
        ctx.json,
        json!({"action": "add_tag", "name": label.to_lowercase()}),
        &format!("Added tag {label}"),
    );
    Ok(())
}

fn mark_changed(conn: &Connection, tag: &Tag) -> Result<()> {
    conn.execute(
        "UPDATE TagModel SET status = ? WHERE _Id = ?",
        params![ctx::bump_int_status(tag.status), tag.local_id],
    )?;
    Ok(())
}

/// Rewrite tag names inside every task that carries `from`.
fn retag_tasks(conn: &Connection, env: &WriteEnv, from: &str, to: Option<&str>) -> Result<usize> {
    let rows: Vec<(String, String)> = {
        let mut stmt = conn.prepare("SELECT id, tag FROM TaskModel WHERE tag LIKE ?")?;
        stmt.query_map([format!("%\"{from}\"%")], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let mut n = 0;
    for (sid, tag_json) in rows {
        let mut tags = model::parse_json_list(Some(&tag_json));
        if !tags.iter().any(|t| t.eq_ignore_ascii_case(from)) {
            continue;
        }
        let mut out: Vec<String> = Vec::new();
        for t in tags.drain(..) {
            let v = if t.eq_ignore_ascii_case(from) {
                to.map(str::to_string)
            } else {
                Some(t)
            };
            if let Some(v) = v
                && !out.contains(&v)
            {
                out.push(v);
            }
        }
        let v = if out.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&out)?)
        };
        conn.execute(
            "UPDATE TaskModel SET tag = ?, modifiedTime = ? WHERE id = ?",
            params![v, ctx::now_str(), sid],
        )?;
        ctx::add_sync_status(conn, env, &sid, sync::MODIFY)?;
        n += 1;
    }
    Ok(n)
}

pub fn rename(ctx: &mut Ctx, from: &str, to: &str) -> Result<()> {
    let tag = find(&ctx.conn, from)?;
    let new_label = to.trim_start_matches('#').trim().to_string();
    let new_name = new_label.to_lowercase();
    if new_name != tag.name && find(&ctx.conn, &new_name).is_ok() {
        bail!(
            "Tag '{new_name}' already exists; use `tags merge {} {new_name}`",
            tag.name
        );
    }
    let n = ctx.write(|conn, env| {
        conn.execute(
            "UPDATE TagModel SET name = ?, label = ?, pinyin = ?, inits = ? WHERE _Id = ?",
            params![
                new_name,
                new_label,
                new_name,
                new_name.chars().next().map(String::from),
                tag.local_id
            ],
        )?;
        conn.execute(
            "UPDATE TagModel SET parent = ? WHERE parent = ?",
            params![new_name, tag.name],
        )?;
        mark_changed(conn, &tag)?;
        retag_tasks(conn, env, &tag.name, Some(&new_name))
    })?;
    out::done(
        ctx.json,
        json!({"action": "rename_tag", "from": tag.name, "to": new_name, "tasks": n}),
        &format!("Renamed tag {} -> {new_name} ({n} task(s))", tag.name),
    );
    Ok(())
}

pub fn merge(ctx: &mut Ctx, from: &str, into: &str) -> Result<()> {
    let src = find(&ctx.conn, from)?;
    let dst = find(&ctx.conn, into)?;
    let n = ctx.write(|conn, env| {
        let n = retag_tasks(conn, env, &src.name, Some(&dst.name))?;
        delete_tag_row(conn, &src)?;
        Ok(n)
    })?;
    out::done(
        ctx.json,
        json!({"action": "merge_tag", "from": src.name, "into": dst.name, "tasks": n}),
        &format!("Merged {} into {} ({n} task(s))", src.name, dst.name),
    );
    Ok(())
}

fn delete_tag_row(conn: &Connection, tag: &Tag) -> Result<()> {
    if tag.status == Some(0) {
        conn.execute("DELETE FROM TagModel WHERE _Id = ?", [tag.local_id])?;
    } else {
        conn.execute(
            "UPDATE TagModel SET deleted = 1, status = -1 WHERE _Id = ?",
            [tag.local_id],
        )?;
    }
    conn.execute(
        "UPDATE TagModel SET parent = NULL WHERE parent = ?",
        [&tag.name],
    )?;
    Ok(())
}

pub fn delete(ctx: &mut Ctx, key: &str) -> Result<()> {
    let tag = find(&ctx.conn, key)?;
    let n = ctx.write(|conn, env| {
        let n = retag_tasks(conn, env, &tag.name, None)?;
        delete_tag_row(conn, &tag)?;
        Ok(n)
    })?;
    out::done(
        ctx.json,
        json!({"action": "delete_tag", "name": tag.name, "tasks": n}),
        &format!("Deleted tag {} (removed from {n} task(s))", tag.name),
    );
    Ok(())
}

pub fn edit(
    ctx: &mut Ctx,
    key: &str,
    color: Option<&str>,
    parent: Option<&str>,
    no_parent: bool,
) -> Result<()> {
    let tag = find(&ctx.conn, key)?;
    let parent = parent
        .map(|p| find(&ctx.conn, p).map(|t| t.name))
        .transpose()?;
    if parent.as_deref() == Some(tag.name.as_str()) {
        bail!("A tag cannot be its own parent");
    }
    ctx.write(|conn, _| {
        if let Some(c) = color {
            conn.execute(
                "UPDATE TagModel SET color = ? WHERE _Id = ?",
                params![c, tag.local_id],
            )?;
        }
        if let Some(p) = &parent {
            conn.execute(
                "UPDATE TagModel SET parent = ? WHERE _Id = ?",
                params![p, tag.local_id],
            )?;
        }
        if no_parent {
            conn.execute(
                "UPDATE TagModel SET parent = NULL WHERE _Id = ?",
                [tag.local_id],
            )?;
        }
        mark_changed(conn, &tag)
    })?;
    out::done(
        ctx.json,
        json!({"action": "edit_tag", "name": tag.name}),
        &format!("Updated tag {}", tag.name),
    );
    Ok(())
}

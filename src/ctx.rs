//! Shared database context: connection handling, the logged-in user, and the
//! small helpers every write path needs (ids, timestamps, sync markers).

use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDateTime};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// TickTick spaces sort orders by 2^28.
pub const SORT_STEP: i64 = 268_435_456;

/// SyncStatusModel.Type values (ticktick_WPF.Resource.Constants).
pub mod sync {
    pub const MODIFY: i64 = 0;
    pub const MOVE: i64 = 2;
    pub const CREATE: i64 = 4;
    pub const TRASH: i64 = 5;
    pub const DELETE_FOREVER: i64 = 6;
    pub const RESTORE: i64 = 7;
}

pub struct Ctx {
    pub conn: Connection,
    pub db_path: PathBuf,
    pub user_id: String,
    pub inbox_id: String,
    pub time_zone: String,
    pub json: bool,
    pub dry_run: bool,
}

pub struct OpenOptions {
    pub write: bool,
    pub allow_running: bool,
    pub backup: bool,
    pub json: bool,
    pub dry_run: bool,
}

impl Ctx {
    pub fn open(db_path: &Path, opts: &OpenOptions) -> Result<Self> {
        if !db_path.exists() {
            bail!("Database file not found: {}", db_path.display());
        }
        if opts.write && !opts.dry_run {
            if !opts.allow_running && crate::app::is_running() {
                bail!(
                    "TickTick appears to be running. It keeps every task in memory and writes whole \
                     rows back, so edits made now can be overwritten and will not show until it restarts.\n\
                     Close it first (`ticktick-cli app stop`), or pass --allow-running to write anyway."
                );
            }
            if opts.backup {
                daily_backup(db_path)?;
            }
        }
        let flags = if opts.write {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
        };
        let conn = Connection::open_with_flags(db_path, flags)
            .with_context(|| format!("Failed to open database: {}", db_path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;

        let (user_id, inbox_id) = load_user(&conn)?;
        let time_zone = load_time_zone(&conn, db_path, &user_id);

        Ok(Self {
            conn,
            db_path: db_path.to_path_buf(),
            user_id,
            inbox_id,
            time_zone,
            json: opts.json,
            dry_run: opts.dry_run,
        })
    }

    /// Run `f` inside a transaction. With --dry-run the transaction is rolled back.
    pub fn write<T>(&mut self, f: impl FnOnce(&Connection, &WriteEnv) -> Result<T>) -> Result<T> {
        let env = WriteEnv {
            user_id: self.user_id.clone(),
            time_zone: self.time_zone.clone(),
        };
        let tx = self.conn.transaction()?;
        ensure_inbox_row(&tx, &self.user_id, &self.inbox_id)?;
        let out = f(&tx, &env)?;
        if self.dry_run {
            tx.rollback()?;
            eprintln!("(dry run: changes rolled back)");
        } else {
            tx.commit()?;
        }
        Ok(out)
    }
}

/// Values a write needs, detached from the connection borrow.
pub struct WriteEnv {
    pub user_id: String,
    pub time_zone: String,
}

fn load_user(conn: &Connection) -> Result<(String, String)> {
    let from_settings: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT LoginUserId, InServerBoxId FROM CommonSettings ORDER BY _Id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap_or(None);
    let mut user_id = from_settings
        .as_ref()
        .and_then(|s| s.0.clone())
        .unwrap_or_default();
    let mut inbox_id = from_settings.and_then(|s| s.1).unwrap_or_default();

    if user_id.is_empty()
        && let Some((u, i)) = conn
            .query_row("SELECT userId, inboxId FROM UserModel LIMIT 1", [], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .optional()
            .unwrap_or(None)
    {
        user_id = u;
        if inbox_id.is_empty() {
            inbox_id = i.unwrap_or_default();
        }
    }
    // An existing inbox row is authoritative.
    let stored: Option<String> = conn
        .query_row(
            "SELECT id FROM ProjectModel WHERE Isinbox = 1 AND (userid = ? OR userid IS NULL)
             AND (delete_status IS NULL OR delete_status = 0) LIMIT 1",
            [&user_id],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None);
    if let Some(s) = stored.filter(|s: &String| !s.is_empty()) {
        inbox_id = s;
    }
    if inbox_id.is_empty() {
        // Utils.GetInboxId()
        inbox_id = format!("inbox{user_id}");
    }
    if user_id.is_empty() {
        bail!(
            "No logged-in TickTick user found in this database (CommonSettings/UserModel are empty)"
        );
    }
    Ok((user_id, inbox_id))
}

/// The app hides inbox tasks unless an inbox ProjectModel row exists. A synced
/// profile always has one; create it like Utils.CreatInbox for profiles that do not.
fn ensure_inbox_row(conn: &Connection, user_id: &str, inbox_id: &str) -> Result<()> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM ProjectModel WHERE Isinbox = 1 OR id = ? LIMIT 1",
            [inbox_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        conn.execute(
            "INSERT INTO ProjectModel (id, userid, name, Isinbox, Ishide, muted, transferred, needPullRemote,
                delete_status, isOwner, inAll, openToTeam, needAudit, sortOrder, userCount, closed, viewMode, kind, ShowAddColumn)
             VALUES (?, ?, 'Inbox', 1, 0, 0, 0, 0, 0, 1, 1, 0, 0, ?, 1, 0, 'list', 'TASK', 1)",
            params![inbox_id, user_id, i64::MIN],
        )?;
    }
    Ok(())
}

/// The IANA zone name the app stamps on new tasks. The app keeps a
/// `TimeZoneDict.txt` ("Windows name:IANA name") next to the database.
fn load_time_zone(conn: &Connection, db_path: &Path, user_id: &str) -> String {
    if let Some(dir) = db_path.parent()
        && let Ok(text) = std::fs::read_to_string(dir.join("TimeZoneDict.txt"))
    {
        let local: Option<String> = conn
            .query_row(
                "SELECT LocalTimeZone FROM CommonSettings LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None)
            .flatten();
        let pairs: Vec<(&str, &str)> = text
            .lines()
            .filter_map(|l| l.split_once(':'))
            .map(|(a, b)| (a.trim(), b.trim()))
            .filter(|(a, b)| !a.is_empty() && !b.is_empty())
            .collect();
        if let Some(local) = local
            && let Some((_, iana)) = pairs.iter().find(|(w, _)| *w == local)
        {
            return iana.to_string();
        }
        if let Some((_, iana)) = pairs.first() {
            return iana.to_string();
        }
    }
    let most_common: Option<String> = conn
        .query_row(
            "SELECT timeZone FROM TaskModel WHERE userId = ? AND timeZone IS NOT NULL AND timeZone <> ''
             GROUP BY timeZone ORDER BY COUNT(*) DESC LIMIT 1",
            [user_id],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None);
    most_common
        .or_else(|| iana_time_zone::get_timezone().ok())
        .unwrap_or_else(|| "UTC".to_string())
}

/// Snapshot the DB once per day before the first write, using SQLite's
/// online backup so the WAL is included.
fn daily_backup(db_path: &Path) -> Result<()> {
    let stamp = Local::now().format("%Y%m%d");
    let target = db_path.with_file_name(format!(
        "{}.cli-backup-{}",
        db_path.file_name().unwrap_or_default().to_string_lossy(),
        stamp
    ));
    if target.exists() {
        return Ok(());
    }
    backup_to(db_path, &target)?;
    eprintln!("Backed up database to {}", target.display());
    Ok(())
}

pub fn backup_to(db_path: &Path, target: &Path) -> Result<()> {
    let src = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut dst = Connection::open(target)?;
    let backup = rusqlite::backup::Backup::new(&src, &mut dst)?;
    backup
        .run_to_completion(256, std::time::Duration::from_millis(5), None)
        .with_context(|| format!("Backup to {} failed", target.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Ids and time
// ---------------------------------------------------------------------------

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A 24-hex-char MongoDB-style ObjectId, the format the app uses for every sid.
pub fn new_object_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    let rand = h.finish();
    let counter = COUNTER
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add((rand >> 40) as u32)
        & 0x00ff_ffff;
    format!("{:08x}{:010x}{:06x}", secs, rand & 0xff_ffff_ffff, counter)
}

pub const DT_FMT: &str = "%Y-%m-%d %H:%M:%S";

pub fn now_local() -> NaiveDateTime {
    let n = Local::now().naive_local();
    n.with_nanosecond_zero()
}

pub fn fmt_dt(dt: &NaiveDateTime) -> String {
    dt.format(DT_FMT).to_string()
}

pub fn now_str() -> String {
    fmt_dt(&now_local())
}

/// .NET `DateTime.Now.Ticks` (100ns units since 0001-01-01, local wall clock).
pub fn now_ticks() -> i64 {
    let n = Local::now().naive_local();
    let epoch = chrono::NaiveDate::from_ymd_opt(1, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let d = n - epoch;
    d.num_seconds() * 10_000_000 + i64::from(d.subsec_nanos()) / 100
}

trait ZeroNanos {
    fn with_nanosecond_zero(self) -> Self;
}

impl ZeroNanos for NaiveDateTime {
    fn with_nanosecond_zero(self) -> Self {
        use chrono::Timelike;
        self.with_nanosecond(0).unwrap_or(self)
    }
}

// ---------------------------------------------------------------------------
// Sync markers (mirrors SyncStatusDao so a later sync pushes CLI changes)
// ---------------------------------------------------------------------------

pub fn add_sync_status(conn: &Connection, env: &WriteEnv, entity: &str, kind: i64) -> Result<()> {
    if kind == sync::CREATE {
        let n = conn.execute(
            "UPDATE SyncStatusModel SET ModifyPoint = ? WHERE EntityId = ? AND Type = ? AND UserId = ?",
            params![now_ticks(), entity, kind, env.user_id],
        )?;
        if n > 0 {
            return Ok(());
        }
    } else {
        let existing: Vec<(i64, i64)> = {
            let mut stmt = conn.prepare(
                "SELECT _Id, Type FROM SyncStatusModel
                 WHERE EntityId = ? AND UserId = ? AND Type <> 2 AND Type <> 16",
            )?;
            stmt.query_map(params![entity, env.user_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        // A pending create already covers later edits.
        if let Some((id, _)) = existing
            .iter()
            .find(|(_, t)| *t == sync::CREATE || *t == kind)
        {
            conn.execute(
                "UPDATE SyncStatusModel SET ModifyPoint = ? WHERE _Id = ?",
                params![now_ticks(), id],
            )?;
            return Ok(());
        }
    }
    conn.execute(
        "INSERT INTO SyncStatusModel (UserId, EntityId, Type, ModifyPoint) VALUES (?, ?, ?, ?)",
        params![env.user_id, entity, kind, now_ticks()],
    )?;
    Ok(())
}

pub fn remove_sync_status(
    conn: &Connection,
    env: &WriteEnv,
    entity: &str,
    kind: i64,
) -> Result<()> {
    conn.execute(
        "DELETE FROM SyncStatusModel WHERE EntityId = ? AND Type = ? AND UserId = ?",
        params![entity, kind, env.user_id],
    )?;
    Ok(())
}

pub fn add_move_status(
    conn: &Connection,
    env: &WriteEnv,
    entity: &str,
    from: &str,
    kind: i64,
) -> Result<()> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT _Id FROM SyncStatusModel WHERE EntityId = ? AND Type = ? AND UserId = ?",
            params![entity, kind, env.user_id],
            |r| r.get(0),
        )
        .optional()?;
    match existing {
        // Keep the original source project so the server sees one move.
        Some(id) => {
            conn.execute(
                "UPDATE SyncStatusModel SET ModifyPoint = ? WHERE _Id = ?",
                params![now_ticks(), id],
            )?;
        }
        None => {
            conn.execute(
                "INSERT INTO SyncStatusModel (UserId, EntityId, MoveFromId, Type, ModifyPoint)
                 VALUES (?, ?, ?, ?, ?)",
                params![env.user_id, entity, from, kind, now_ticks()],
            )?;
        }
    }
    Ok(())
}

pub fn add_set_parent_status(
    conn: &Connection,
    env: &WriteEnv,
    task: &str,
    old_parent: &str,
) -> Result<()> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM SyncStatusModel WHERE EntityId = ? AND Type = 16 AND UserId = ?",
        params![task, env.user_id],
        |r| r.get(0),
    )?;
    if n == 0 {
        conn.execute(
            "INSERT INTO SyncStatusModel (UserId, EntityId, OldParentId, Type, ModifyPoint)
             VALUES (?, ?, ?, 16, ?)",
            params![env.user_id, task, old_parent, now_ticks()],
        )?;
    }
    Ok(())
}

/// Tags, habits, check-ins and focus records use 0=new, 1=changed, 2=synced, -1=deleted.
pub fn bump_int_status(current: Option<i64>) -> i64 {
    if current.unwrap_or(0) == 0 { 0 } else { 1 }
}

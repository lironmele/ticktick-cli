//! End-to-end tests: build an empty TickTick 8.2.1.5 profile from the schema
//! fixture, drive the binary, and check the rows the desktop app will read.

use rusqlite::Connection;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

struct Db {
    dir: PathBuf,
}

impl Db {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ticktick-cli-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join("TickTick.db")).unwrap();
        conn.execute_batch(include_str!("fixtures/schema.sql"))
            .unwrap();
        conn.execute_batch(
            "INSERT INTO CommonSettings (InBoxId, LoginUserId, InServerBoxId, LocalTimeZone) VALUES (0, '1001', 'inbox1001', 'UTC');
             INSERT INTO UserModel (userId, username, inboxId, pro) VALUES ('1001', 'test@example.test', 'inbox1001', 1);",
        )
        .unwrap();
        std::fs::write(dir.join("TimeZoneDict.txt"), "UTC:Etc/UTC\n").unwrap();
        Db { dir }
    }

    fn path(&self) -> PathBuf {
        self.dir.join("TickTick.db")
    }

    fn conn(&self) -> Connection {
        Connection::open(self.path()).unwrap()
    }

    fn run(&self, args: &[&str]) -> (bool, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_ticktick-cli"))
            .arg("--db")
            .arg(self.path())
            .args(["--allow-running", "--no-backup"])
            .args(args)
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }

    /// Run with --json and parse stdout; panics on failure.
    fn json(&self, args: &[&str]) -> Value {
        let mut a = vec!["--json"];
        a.extend_from_slice(args);
        let (ok, out, err) = self.run(&a);
        assert!(ok, "command {args:?} failed: {err}{out}");
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("bad json from {args:?}: {e}\n{out}"))
    }

    fn fails(&self, args: &[&str]) -> String {
        let (ok, out, err) = self.run(args);
        assert!(!ok, "command {args:?} unexpectedly succeeded: {out}");
        err
    }

    fn scalar<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        self.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn add_task_writes_app_compatible_row() {
    let db = Db::new("add");
    let t = db.json(&[
        "tasks",
        "add",
        "Write report",
        "--priority",
        "high",
        "-t",
        "Work,urgent",
        "-d",
        "2030-01-15 14:00",
        "--duration",
        "1h",
        "-r",
        "-15m",
        "-c",
        "body",
    ]);
    assert_eq!(t["title"], "Write report");
    assert_eq!(t["priority"], "high");
    assert_eq!(t["project"], "Inbox");
    assert_eq!(t["tags"], serde_json::json!(["work", "urgent"]));
    assert_eq!(t["start"], "2030-01-15 14:00:00");
    assert_eq!(t["due"], "2030-01-15 15:00:00");
    assert_eq!(t["all_day"], false);
    assert_eq!(t["reminders"], serde_json::json!(["TRIGGER:-PT15M"]));

    let sid = t["sid"].as_str().unwrap();
    assert_eq!(sid.len(), 24);
    let conn = db.conn();
    let (prio, user, tz, kind, tag, rc): (i64, String, String, String, String, i64) = conn
        .query_row(
            "SELECT priority, userId, timeZone, kind, tag, reminderCount FROM TaskModel WHERE id = ?",
            [sid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .unwrap();
    assert_eq!(
        (prio, user.as_str(), tz.as_str(), kind.as_str(), rc),
        (5, "1001", "Etc/UTC", "TEXT", 1)
    );
    assert_eq!(tag, r#"["work","urgent"]"#);
    // a pending create marker so a later sync pushes it
    let n: i64 = db.scalar(&format!(
        "SELECT COUNT(*) FROM SyncStatusModel WHERE EntityId = '{sid}' AND Type = 4"
    ));
    assert_eq!(n, 1);
    // tags were created, label keeps case
    let label: String = db.scalar("SELECT label FROM TagModel WHERE name = 'work'");
    assert_eq!(label, "Work");
    // the inbox row the app needs exists
    let inbox: i64 = db.scalar("SELECT COUNT(*) FROM ProjectModel WHERE Isinbox = 1 AND id = 'inbox1001' AND delete_status = 0");
    assert_eq!(inbox, 1);
}

#[test]
fn all_day_spans_and_date_edits() {
    let db = Db::new("dates");
    db.json(&[
        "tasks",
        "add",
        "Trip",
        "-d",
        "2030-03-02",
        "--end",
        "2030-03-05",
    ]);
    let t = db.json(&["tasks", "show", "Trip"]);
    assert_eq!(t["all_day"], true);
    assert_eq!(t["start"], "2030-03-02 00:00:00");
    assert_eq!(t["due"], "2030-03-06 00:00:00", "all-day end is exclusive");
    // moving the start keeps the span
    let t = db.json(&["tasks", "edit", "Trip", "--date", "2030-03-10"]);
    assert_eq!(t["start"], "2030-03-10 00:00:00");
    assert_eq!(t["due"], "2030-03-14 00:00:00");
    // single all-day day has no end
    db.json(&["tasks", "add", "Day", "-d", "2030-03-02"]);
    let t = db.json(&["tasks", "show", "Day"]);
    assert_eq!(t["due"], Value::Null);
    let t = db.json(&["tasks", "edit", "Day", "--clear-date"]);
    assert_eq!(t["start"], Value::Null);
}

#[test]
fn list_filters() {
    let db = Db::new("filters");
    db.json(&["projects", "add", "Work"]);
    db.json(&[
        "tasks",
        "add",
        "A",
        "-p",
        "Work",
        "-t",
        "x",
        "--priority",
        "medium",
        "-d",
        "today",
    ]);
    db.json(&["tasks", "add", "B", "-d", "2001-01-01"]);
    db.json(&["tasks", "add", "C"]);
    db.json(&["tasks", "add", "D needle", "-c", "hay"]);
    db.json(&["tasks", "complete", "C"]);
    let titles = |v: Value| -> Vec<String> {
        let mut t: Vec<String> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["title"].as_str().unwrap().to_string())
            .collect();
        t.sort();
        t
    };
    assert_eq!(titles(db.json(&["tasks", "list"])), ["A", "B", "D needle"]);
    assert_eq!(
        titles(db.json(&["tasks", "list", "-a"])),
        ["A", "B", "C", "D needle"]
    );
    assert_eq!(titles(db.json(&["tasks", "list", "-p", "work"])), ["A"]);
    assert_eq!(titles(db.json(&["tasks", "list", "-t", "x"])), ["A"]);
    assert_eq!(
        titles(db.json(&["tasks", "list", "--priority", "medium"])),
        ["A"]
    );
    assert_eq!(
        titles(db.json(&["tasks", "list", "--due", "overdue"])),
        ["B"]
    );
    assert_eq!(
        titles(db.json(&["tasks", "list", "--due", "today"])),
        ["A", "B"],
        "today includes overdue"
    );
    assert_eq!(
        titles(db.json(&["tasks", "list", "--due", "nodate"])),
        ["D needle"]
    );
    assert_eq!(
        titles(db.json(&["tasks", "list", "--status", "completed"])),
        ["C"]
    );
    assert_eq!(titles(db.json(&["search", "needle"])), ["D needle"]);
    assert_eq!(
        titles(db.json(&["tasks", "list", "-q", "hay"])),
        ["D needle"]
    );
    let agenda = db.json(&["agenda", "--days", "2"]);
    assert_eq!(agenda["overdue"].as_array().unwrap().len(), 1);
}

#[test]
fn complete_reopen_wontdo_and_subtasks() {
    let db = Db::new("status");
    db.json(&["tasks", "add", "Parent"]);
    let sub = db.json(&["tasks", "add", "Child", "--parent", "Parent"]);
    let parent = db.json(&["tasks", "show", "Parent"]);
    assert_eq!(parent["subtasks"][0]["title"], "Child");
    let children: String = db.scalar("SELECT childrenString FROM TaskModel WHERE title = 'Parent'");
    assert!(children.contains(sub["sid"].as_str().unwrap()));

    db.json(&["tasks", "complete", "Parent"]);
    let st: i64 = db.scalar("SELECT status FROM TaskModel WHERE title = 'Child'");
    assert_eq!(st, 2, "completing a parent completes its subtasks");
    let completed_by: String =
        db.scalar("SELECT completedUserId FROM TaskModel WHERE title = 'Parent'");
    assert_eq!(completed_by, "1001");

    db.json(&["tasks", "reopen", "Child"]);
    let st: i64 = db.scalar("SELECT status FROM TaskModel WHERE title = 'Parent'");
    assert_eq!(st, 0, "reopening a subtask reopens the parent");

    db.json(&["tasks", "wontdo", "Child"]);
    let st: i64 = db.scalar("SELECT status FROM TaskModel WHERE title = 'Child'");
    assert_eq!(st, -1);

    db.json(&["tasks", "edit", "Child", "--no-parent"]);
    let p: Option<String> = db.scalar("SELECT parentId FROM TaskModel WHERE title = 'Child'");
    assert_eq!(p, None);
    let children: Option<String> =
        db.scalar("SELECT childrenString FROM TaskModel WHERE title = 'Parent'");
    assert_eq!(children, None);
}

#[test]
fn repeating_task_completion() {
    let db = Db::new("repeat");
    db.json(&[
        "tasks",
        "add",
        "Water",
        "-d",
        "2030-05-01 08:00",
        "--repeat",
        "every 3 days",
        "-i",
        "a",
    ]);
    db.json(&["items", "check", "Water", "1"]);
    let r = db.json(&["tasks", "complete", "Water"]);
    assert_eq!(r[0]["start"], "2030-05-04 08:00:00");
    assert_eq!(r[0]["status"], "open");
    assert_eq!(r[0]["closed_occurrence"]["status"], "completed");
    assert_eq!(r[0]["closed_occurrence"]["start"], "2030-05-01 08:00:00");
    assert_eq!(r[0]["closed_occurrence"]["repeat"], Value::Null);
    // the original's checklist resets, the closed copy keeps it
    let open_items: i64 = db.scalar(
        "SELECT COUNT(*) FROM TaskDetailItemModel i JOIN TaskModel t ON t.id = i.TaskServerId WHERE t.status = 0 AND i.status = 0",
    );
    assert_eq!(open_items, 1);
    let rt: String = db.scalar("SELECT repeatTaskId FROM TaskModel WHERE status = 2");
    assert_eq!(rt, r[0]["sid"].as_str().unwrap());

    // COUNT is consumed and a finished series closes outright
    db.json(&[
        "tasks",
        "add",
        "Twice",
        "-d",
        "2030-05-01",
        "--repeat",
        "RRULE:FREQ=WEEKLY;COUNT=2",
    ]);
    let r = db.json(&["tasks", "complete", "Twice"]);
    assert_eq!(r[0]["repeat"], "RRULE:FREQ=WEEKLY;COUNT=1");
    let r = db.json(&["tasks", "complete", "Twice"]);
    assert_eq!(r[0]["status"], "completed");

    // whole series
    db.json(&[
        "tasks",
        "add",
        "Daily",
        "-d",
        "2030-05-01",
        "--repeat",
        "daily",
    ]);
    let r = db.json(&["tasks", "complete", "Daily", "--whole-series"]);
    assert_eq!(r[0]["status"], "completed");
}

#[test]
fn checklist_items() {
    let db = Db::new("items");
    db.json(&["tasks", "add", "Pack", "-c", "desc"]);
    let t = db.json(&["items", "add", "Pack", "socks", "shirts", "passport"]);
    assert_eq!(t["kind"], "checklist");
    assert_eq!(
        t["content"], "desc",
        "description moves to desc for checklists"
    );
    let t = db.json(&["items", "check", "Pack", "1", "shirts"]);
    assert_eq!(t["progress"], 66);
    let t = db.json(&["items", "add", "Pack", "charger", "--at", "1"]);
    assert_eq!(t["items"][0]["title"], "charger");
    let t = db.json(&["items", "edit", "Pack", "charger", "--title", "usb charger"]);
    assert_eq!(t["items"][0]["title"], "usb charger");
    let t = db.json(&["items", "check", "Pack", "1", "4", "--complete-task"]);
    assert_eq!(t["status"], "completed");
    db.json(&["tasks", "reopen", "Pack"]);
    let sub = db.json(&["items", "promote", "Pack", "passport"]);
    assert_eq!(sub["parent_sid"], t["sid"]);
    let t = db.json(&["items", "delete", "Pack", "1", "2", "3"]);
    assert_eq!(t["kind"], "task");
    assert_eq!(t["content"], "desc");
}

#[test]
fn move_trash_restore_duplicate() {
    let db = Db::new("move");
    db.json(&["projects", "add", "Home"]);
    db.json(&[
        "tasks",
        "add",
        "Thing",
        "-i",
        "x",
        "-d",
        "2030-01-01",
        "-r",
        "9:00",
    ]);
    db.json(&["tasks", "move", "Thing", "-p", "Home"]);
    let t = db.json(&["tasks", "show", "Thing"]);
    assert_eq!(t["project"], "Home");

    let d = db.json(&["tasks", "duplicate", "Thing"]);
    assert_eq!(d["items"].as_array().unwrap().len(), 1);
    assert_eq!(d["reminders"].as_array().unwrap().len(), 1);
    assert_ne!(d["sid"], t["sid"]);

    db.json(&["tasks", "delete", t["sid"].as_str().unwrap()]);
    assert_eq!(db.json(&["trash", "list"]).as_array().unwrap().len(), 1);
    db.json(&["tasks", "restore", t["sid"].as_str().unwrap()]);
    assert_eq!(db.json(&["trash", "list"]).as_array().unwrap().len(), 0);

    db.json(&["tasks", "delete", d["sid"].as_str().unwrap()]);
    db.json(&["trash", "empty"]);
    let del: i64 = db.scalar(&format!(
        "SELECT deleted FROM TaskModel WHERE id = '{}'",
        d["sid"].as_str().unwrap()
    ));
    assert_eq!(del, 2);
}

#[test]
fn projects_folders_sections() {
    let db = Db::new("projects");
    db.json(&["folders", "add", "Life"]);
    let p = db.json(&[
        "projects", "add", "Home", "--folder", "Life", "--color", "#35D870",
    ]);
    assert_eq!(p["folder"], "Life");
    let status: String = db.scalar("SELECT sync_status FROM ProjectModel WHERE name = 'Home'");
    assert_eq!(status, "SYNC_NEW");
    let ds: i64 = db.scalar("SELECT delete_status FROM ProjectModel WHERE name = 'Home'");
    assert_eq!(ds, 0, "the app only shows projects with delete_status = 0");

    db.json(&["projects", "add", "Board", "--view", "kanban"]);
    db.json(&["sections", "add", "Board", "Doing"]);
    db.json(&["tasks", "add", "Card", "-p", "Board"]);
    let col: String = db.scalar("SELECT c.name FROM TaskModel t JOIN ColumnModel c ON c.id = t.columnId WHERE t.title = 'Card'");
    assert_eq!(col, "Not Sectioned");
    db.json(&["tasks", "edit", "Card", "--column", "doing"]);
    db.json(&["sections", "delete", "Board", "Doing"]);
    let col: String = db.scalar("SELECT c.name FROM TaskModel t JOIN ColumnModel c ON c.id = t.columnId WHERE t.title = 'Card'");
    assert_eq!(col, "Not Sectioned");

    db.json(&["projects", "edit", "Home", "--name", "House", "--no-folder"]);
    db.json(&["projects", "archive", "House"]);
    let listed = db.json(&["projects", "list"]);
    assert!(
        !listed
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "House")
    );
    db.json(&["projects", "unarchive", "House"]);
    db.json(&["tasks", "add", "In house", "-p", "House"]);
    db.json(&["projects", "delete", "House", "--move-tasks-to", "inbox"]);
    let t = db.json(&["tasks", "show", "In house"]);
    assert_eq!(t["project"], "Inbox");
    db.json(&["folders", "delete", "Life"]);
    assert_eq!(db.json(&["folders", "list"]).as_array().unwrap().len(), 0);
}

#[test]
fn tags() {
    let db = Db::new("tags");
    db.json(&["tasks", "add", "A", "-t", "old,keep"]);
    db.json(&["tasks", "add", "B", "-t", "old"]);
    db.json(&["tags", "rename", "old", "New"]);
    let tag: String = db.scalar("SELECT tag FROM TaskModel WHERE title = 'B'");
    assert_eq!(tag, r#"["new"]"#);
    db.json(&["tags", "merge", "keep", "new"]);
    let tag: String = db.scalar("SELECT tag FROM TaskModel WHERE title = 'A'");
    assert_eq!(tag, r#"["new"]"#);
    db.json(&["tags", "add", "child", "--parent", "new"]);
    db.json(&["tags", "delete", "new"]);
    let tag: Option<String> = db.scalar("SELECT tag FROM TaskModel WHERE title = 'A'");
    assert_eq!(tag, None);
    let names: Vec<String> = db
        .json(&["tags", "list"])
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["child"]);
    db.fails(&["tasks", "add", "C", "-t", "bad tag"]);
}

#[test]
fn habits_and_focus() {
    let db = Db::new("habits");
    db.json(&["habits", "add", "Meditate"]);
    db.json(&["habits", "add", "Water", "--goal", "8", "--unit", "cups"]);
    db.json(&["habits", "checkin", "Meditate"]);
    let r = db.json(&["habits", "checkin", "Water", "--value", "3"]);
    assert_eq!(r["state"], "partial");
    let r = db.json(&["habits", "checkin", "Water", "--value", "5"]);
    assert_eq!(r["state"], "done");
    db.json(&["habits", "fail", "Meditate", "--date", "yesterday"]);
    db.json(&["habits", "note", "Meditate", "calm"]);
    let h = db.json(&["habits", "show", "Meditate", "--days", "2"]);
    assert_eq!(h["recent"][0]["state"], "failed");
    assert_eq!(h["recent"][1]["state"], "done");
    assert_eq!(h["streak"], 1);
    let v: f64 = db.scalar("SELECT Value FROM HabitCheckInModel h JOIN HabitModel m ON m.Id = h.HabitId WHERE m.Name = 'Water'");
    assert_eq!(v, 8.0);
    db.json(&["habits", "reset", "Meditate"]);
    db.json(&["habits", "archive", "Water"]);
    assert_eq!(db.json(&["habits", "list"]).as_array().unwrap().len(), 1);
    db.fails(&["habits", "checkin", "Meditate", "--date", "+2d"]);

    db.json(&["tasks", "add", "Focus task"]);
    let p = db.json(&[
        "pomos",
        "add",
        "--start",
        "2030-01-01 09:00",
        "--duration",
        "25m",
        "-t",
        "Focus task",
    ]);
    let list = db.json(&["pomos", "list", "--from", "2030-01-01"]);
    assert_eq!(list[0]["task"], "Focus task");
    assert_eq!(list[0]["seconds"], 1500);
    db.json(&["pomos", "delete", p["sid"].as_str().unwrap()]);
    assert_eq!(
        db.json(&["pomos", "list", "--from", "2030-01-01"])
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn safety() {
    let db = Db::new("safety");
    // dry run leaves no trace
    let (ok, _, err) = db.run(&["--dry-run", "tasks", "add", "Ghost"]);
    assert!(ok);
    assert!(err.contains("dry run"));
    let n: i64 = db.scalar("SELECT COUNT(*) FROM TaskModel");
    assert_eq!(n, 0);
    // raw SQL is read-only unless --write
    db.fails(&["sql", "DELETE FROM TaskModel"]);
    let rows = db.json(&["sql", "SELECT COUNT(*) AS n FROM TaskModel"]);
    assert_eq!(rows[0]["n"], 0);
    // errors are JSON with --json
    let (ok, out, _) = db.run(&["--json", "tasks", "show", "nope"]);
    assert!(!ok);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert!(v["error"].as_str().unwrap().contains("not found"));
    // bad input is rejected before writing
    db.fails(&["tasks", "add", "x", "-d", "someday"]);
    db.fails(&["tasks", "add", "x", "--repeat", "daily"]);
    db.fails(&["tasks", "add", "x", "--priority", "urgent"]);
    let n: i64 = db.scalar("SELECT COUNT(*) FROM TaskModel");
    assert_eq!(n, 0);
    // ambiguous titles must use an id
    db.json(&["tasks", "add", "Same"]);
    db.json(&["tasks", "add", "Same"]);
    let err = db.fails(&["tasks", "complete", "Same"]);
    assert!(err.contains("matches 2 tasks"));
}

#[test]
fn backup_command_and_daily_backup() {
    let db = Db::new("backup");
    let out = db.dir.join("copy.db");
    db.json(&["backup", "--out", out.to_str().unwrap()]);
    assert!(out.exists());
    // daily backup happens on the first write when not disabled
    let st = Command::new(env!("CARGO_BIN_EXE_ticktick-cli"))
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "--allow-running",
            "tasks",
            "add",
            "x",
        ])
        .output()
        .unwrap();
    assert!(st.status.success());
    let backups = std::fs::read_dir(&db.dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("cli-backup-")
        })
        .count();
    assert_eq!(backups, 1);
}

# AGENTS.md

## Using ticktick-cli as an agent

The binary documents itself; nothing else is needed at runtime:

* `ticktick-cli guide`: rules for writing, id formats, date/reminder/repeat syntax, every JSON output shape,
  recipes and limits (source: `src/guide.md`, compiled in).
* `ticktick-cli --json commands`: every command, alias, argument, type, default and allowed value,
  generated from the parser.
* `ticktick-cli <command> --help`: the same per command.

Tests fail if an argument lacks help text or a JSON key the CLI emits is missing from the guide, so keep
`src/guide.md` updated when changing output.

## Why the app must be closed

TickTick for Windows (`ticktick_WPF`, .NET/WPF) reads the whole `TaskModel` table into `TaskCache` at startup and
saves edits by writing whole rows back from that cache (`App.Connection.UpdateAsync(task)`). A row changed behind its
back stays invisible until restart, and the app overwrites it the next time it touches that task.

## How writes are modelled

The rules below come from the decompiled 8.2.1.5 client. Keep them when changing write code:

* Database: SQLite in WAL mode, accessed through sqlite-net with `storeDateTimeAsTicks = false`. Dates are local
  wall-clock text, `yyyy-MM-dd HH:mm:ss`. Booleans are 0/1.
* Ids: `id` columns are 24-hex ObjectIds (`ObjectIdGenerator`). `_Id` is the local autoincrement key.
* User: `CommonSettings.LoginUserId`. Inbox id: `"inbox" + userId` (`Utils.GetInboxId`). The app hides inbox tasks
  unless an `Isinbox = 1` row exists in `ProjectModel`, so the CLI creates one on write if it is missing
  (`Utils.CreatInbox`).
* Sort orders step by 2^28. New tasks go above their siblings (`TaskDefaultModel.AddTo = 0`) or below them.
* Task status: 0 open, 2 completed, -1 won't do. `deleted`: 0, 1 trash, 2 deleted forever. Priority: 0/1/3/5.
* Tags: `TaskModel.tag` is a JSON array of lowercased names. `TagModel.label` keeps the display case.
* Checklist tasks have `kind = 'CHECKLIST'`. Their description lives in `desc`, and the items are
  `TaskDetailItemModel` rows (status 1 = checked). `progress` is the percentage of checked items.
* All-day spans store an exclusive end in `dueDate`. A single all-day day has `dueDate = NULL`.
* Reminders are `TaskReminderModel.trigger` iCalendar values, and `TaskModel.reminderCount` must match the count.
* Completing a repeating task inserts a completed copy (`repeatTaskId` = original) and advances the original's
  `startDate`/`dueDate` by the RRULE, decrementing `COUNT`.
* Sync markers: `SyncStatusModel` rows (Type 0 modify, 2 move, 4 create, 5 trash, 6 delete forever, 7 restore,
  16 set parent). Projects and folders use `sync_status` `SYNC_NEW`/`SYNC_UPDATE`. Columns use `new`/`update`/`delete`.
  Tags, habits, check-ins and focus records use 0 new / 1 changed / 2 synced / -1 deleted.
* Projects are only shown when `delete_status = 0` (NULL hides them) and `closed` is 0 or NULL.

## Development

```bash
cargo build
cargo test          # unit + end-to-end tests; tests build a profile from tests/fixtures/schema.sql
```

Layout:

* `src/main.rs`: clap command tree and dispatch.
* `src/ctx.rs`: opening the DB, running-app guard, backups, transactions, ids, sync markers.
* `src/model.rs`: row types, id resolution, and JSON views.
* `src/tasks.rs`, `items.rs`, `projects.rs`, `tags.rs`, `habits.rs`, `pomos.rs`, `misc.rs`: the commands.
* `src/dates.rs`: parsing of dates, durations, reminders and repeat rules, plus RRULE stepping.
* `src/app.rs`: finding, stopping and starting `TickTick.exe`.

Tags starting with `v` trigger `.github/workflows/release.yml`, which builds Linux, macOS and Windows binaries.

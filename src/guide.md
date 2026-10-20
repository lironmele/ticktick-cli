# ticktick-cli agent guide

ticktick-cli reads and edits the local SQLite database of the TickTick desktop app (`TickTick.db`).
It needs no network or login. This guide is everything an agent needs; for exact flags run
`ticktick-cli commands --json` (the whole command tree) or `ticktick-cli <command> --help`.

## 1. First steps

```
ticktick-cli --json info              # db path, user, inbox sid, time zone, app_running, counts
ticktick-cli --json agenda            # overdue + next 7 days of open tasks
ticktick-cli --json projects list     # project names/ids for -p
ticktick-cli --json tags list
ticktick-cli --json export            # everything (open tasks, projects, tags) in one document
```

The database is found via `--db PATH`, then `$TICKTICK_DB`, then `%APPDATA%\Tick_Tick\TickTick.db`
(macOS `~/Library/Application Support/Tick_Tick/TickTick.db`, Linux `~/.local/share/Tick_Tick/TickTick.db`).

## 2. Rules for writing

* The desktop app caches all tasks in memory and writes whole rows back. **Writes fail while it is running**
  (error mentions "appears to be running"). Options:
  * `--restart-app`: close the app, write, start it again (recommended when the user has it open);
  * `ticktick-cli app stop` before a batch of writes and `ticktick-cli app start` after;
  * `--allow-running`: write anyway (the app will not show the change until restart and may overwrite it; avoid).
* The app only shows CLI changes after it (re)starts.
* Each command is one transaction: it fully applies or not at all.
* `--dry-run` runs the change and rolls it back (prints "(dry run: changes rolled back)" on stderr). Use it to
  validate input. Objects printed after a dry run show the database as it is (unchanged).
* Before the first write of each day a backup `TickTick.db.cli-backup-YYYYMMDD` is made next to the DB
  (`--no-backup` skips it). `ticktick-cli backup` makes one on demand.
* Prefer dedicated commands over `sql --write`; they keep sort orders, sync markers, checklist progress,
  parent/child links and tag rows consistent.

## 3. Output contract

* Always pass `--json` (global, may go anywhere on the command line). Output is one JSON value on stdout.
* Exit code 0 = success. Non-zero = failure; with `--json` stdout is `{"error": "message"}`.
* Read commands return arrays (lists) or objects (show). Write commands return the changed object or a summary
  object with an `"action"` field (see section 6).
* Times are local wall-clock strings `YYYY-MM-DD HH:MM:SS` (the app's own format, no time zone suffix).
* Status strings: `open`, `completed`, `wontdo`. Priority strings: `none`, `low`, `medium`, `high`.
  Kind strings: `task`, `note`, `checklist`.

## 4. Identifiers

| Object | Accepted forms |
| --- | --- |
| task | numeric `id` (local row id from list output), 24-hex `sid`, unique sid prefix (6+ hex chars), or exact title (case-insensitive; open tasks preferred; ambiguous titles are an error listing the ids) |
| project | `inbox`, numeric `id`, `sid`, exact name (case-insensitive), or unique name prefix |
| checklist item | 1-based position within its task, item `sid`, exact title, or unique part of the title (always given together with its task) |
| tag | name with or without `#` (case-insensitive) |
| habit | name (case-insensitive), unique substring, numeric `id`, or `sid` |
| folder / section | name, numeric `id`, or `sid` (sections are always given with their project) |
| focus record | `sid` from `pomos list --json` |

Prefer `id`/`sid` from earlier output over titles. `id` is stable for the life of the row.

## 5. Input syntax

**Dates** (`--date/--due/--start`, `--end`, `--from`, `--to`, `--at`, `--modified-since`, ...):
`today`, `tomorrow`, `yesterday`, `now`, weekday names (`fri`, `next monday`: the next such day after today),
`YYYY-MM-DD`, `YYYY/MM/DD`, offsets `+3d` `-1w` (dates) and `+2h` `+30m` (date-times),
each optionally followed by a time: `tomorrow 9:30`, `fri 5pm`, `2026-11-01 14:00`, ISO `2026-11-01T14:00`.
A bare time (`17:00`) means today. **A date without a time makes an all-day task.**

**Spans**: `--date` is the start; `--end` the end (for all-day tasks the inclusive last day) or `--duration`
(`30m`, `1h30m`, `90`, `1.5h`). Editing `--date` alone keeps an existing span's length.

**Reminders** (`-r`, repeatable): `0` (at start), `-5m`, `-1h`, `-1d`, `-1h30m` (before start);
for all-day tasks `9:00` (on the day) or `-1d@9:00` (the day before); or raw `TRIGGER:-PT15M`.
Reminders need a date.

**Repeat** (`--repeat`): `daily`, `weekly`, `monthly`, `yearly`, `weekdays`, `weekends`, `every 2 days`,
`every 3 weeks`, `every mon,wed,fri`, or a raw RRULE (`RRULE:FREQ=MONTHLY;BYMONTHDAY=1`, `COUNT=`, `UNTIL=`).
`--repeat-from due|completion`. Repeats need a date. Completing a repeating task closes one occurrence
(a completed copy is created, returned as `closed_occurrence`) and moves the task to its next date;
`--whole-series` closes it outright. Rules with BYSETPOS/monthly-by-weekday etc. require `--whole-series`.

**Tags**: `-t a,b` or repeated `-t`; created automatically if missing; stored lowercase, no spaces or
`\ / " ' * ? < > | : , ;`.

**Priority**: `none|low|medium|high` (also `0|1|3|5`).

## 6. JSON shapes

**Task** (`tasks list`, `search`, `agenda`, `trash list`, status changes):
`id, sid, title, status, priority, kind, project_sid, project, tags[], start, due, all_day, repeat,
parent_sid, pinned, trashed, created_time, modified_time, completed_time, sort_order`.
`start` is the task's date (all-day tasks: midnight). `due` is the end of a span (all-day: exclusive midnight
after the last day) or null.

**Full task** (`tasks show`, `tasks add`, `tasks edit`, `tasks duplicate`, `items ...`, `export`): the above plus
`content` (the description, whatever the kind), `raw_content`, `desc`, `items[]` (checklist items),
`reminders[]` (TRIGGER strings), `repeat_from` ("0" due date / "1" completion), `time_zone`, `progress`
(checklist percent), `column_sid`, `column`, `subtasks[]` ({id, sid, title, status}), `ex_dates`.

**Checklist item**: `id, sid, task_sid, title, done, sort_order, date, all_day, completed_time`.

**Project** (`projects list/show/add/edit`): `id` (null for a synthetic inbox), `sid, name, inbox, color,
folder_id, folder, archived, kind (task|note), view_mode (list|kanban|timeline), muted, show_in_all, sort_order,
open_tasks`; `show` adds `sections[]` ({id, sid, name, sort_order}).

**Tag**: `id, sid, name, label, color, parent, open_tasks`.

**Folder**: `id, sid, name, sort_order, projects[]` (names). **Section**: `id, sid, name, sort_order, open_tasks`.

**Habit**: `id, sid, name, type (boolean|count), goal, step, unit, archived, repeat, color, reminders,
encouragement, target_days, created_time, total_checkins, streak, today, recent[]` where
`recent[] = {date, state (done|failed|partial|none), value}`.

**Focus record**: `sid, task_sid, task, project, start, end, seconds, status, type (pomodoro|stopwatch), note`;
with `--summary`: `{date, pomos, sessions, seconds}`.

**Agenda**: `{overdue: [task], days: [{date, tasks: [task]}], undated_open_tasks: n}`.

**Status changes** (`complete`, `wontdo`, `reopen`): array of tasks; repeating tasks also carry
`closed_occurrence` (the completed copy).

**Summaries**: other writes print `{"action": "...", ...}`, e.g. `{"action":"trash","sids":[...]}`,
`{"action":"move","project":...,"sids":[...]}`, `{"action":"rename_tag","from":..,"to":..,"tasks":n}`,
`{"habit":..,"date":..,"state":..}` for check-ins.

**Info**: `db, user_id, username, inbox_sid, time_zone, app_running, open_tasks, completed_tasks,
trashed_tasks, projects, tags, habits, pending_sync_changes`.

## 7. Command map

```
tasks     list | show | add | edit | complete | wontdo | reopen | delete [--forever] | restore | move | duplicate
items     list | add [--at N] | check [--complete-task] | uncheck | edit | delete | promote
projects  list [--all] | show | add | edit | archive | unarchive | delete [--move-tasks-to P]
folders   list | add | rename | delete          sections  list | add | rename | delete [--move-to S]
tags      list | add | rename | merge | edit | delete
habits    list | show | add | edit | checkin [--value N --set] | fail | reset | note | archive | unarchive | delete
pomos     list [--summary] | add | delete       trash  list | empty | restore
agenda | search | info | export | comments | filters | sql [--write] | schema | backup | app status|stop|start|restart
guide | commands
```

Useful `tasks list` filters: `-p PROJECT` (repeatable), `-t TAG` (repeatable, `--all-tags`, `none` = untagged),
`-s open|completed|wontdo|closed|all` (`-a` = all), `-d today|tomorrow|overdue|week|nodate|DATE`
(`today` includes overdue open tasks, like the app), `--from/--to`, `--priority MIN`, `--kind`, `--parent T`,
`--top-level`, `--pinned`, `--repeating`, `-q TEXT`, `--completed-from/--completed-to`, `--modified-since`,
`--include-archived`, `--sort order|date|priority|created|modified|completed|title`, `--reverse`, `-l N`.

## 8. Recipes

```
# what is due
ticktick-cli --json tasks list -d today --sort date
# capture a task for the user who has TickTick open
ticktick-cli --json --restart-app tasks add "Call Bob" -d "tomorrow 10:00" -r -15m -p Work -t phone
# reschedule everything overdue to today (one restart for the batch)
ticktick-cli app stop
for id in $(ticktick-cli --json tasks list -d overdue | jq '.[].id'); do ticktick-cli --json tasks edit $id -d today; done
ticktick-cli app start
# complete, with a note
ticktick-cli --json tasks edit 42 --append "done via agent" && ticktick-cli --json tasks complete 42
# checklist
ticktick-cli --json items add 42 "step one" "step two" && ticktick-cli --json items check 42 1
# weekly review
ticktick-cli --json tasks list -s completed --completed-from -7d
# anything not covered: read-only SQL (see `schema TaskModel`)
ticktick-cli --json sql "SELECT title, startDate FROM TaskModel WHERE status = 0 AND deleted = 0 LIMIT 20"
```

## 9. Limits

* Comments and saved filters are read-only. Attachments, shared-list collaboration, calendar events and
  location reminders are not supported.
* Changes reach TickTick's servers only if this copy of the app ever syncs; the CLI queues them like the app does.
* `app start` looks for TickTick.exe in Program Files, LocalAppData and `%SystemDrive%\TickTick`; otherwise
  pass `--exe PATH` or set `$TICKTICK_EXE`.

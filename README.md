# ticktick-cli

A command-line interface to the **local database of the TickTick desktop app**, with full read **and write** support.
It needs no login and no network, so it also works with an offline or airgapped install. It is built to be driven
by scripts and AI agents (`--json` everywhere, dry runs, safe defaults).

This is a fork of [Lyt99/ticktick-cli](https://github.com/Lyt99/ticktick-cli), which was read-only. Every write here
mirrors what TickTick for Windows 8.2.1.5 does in its own code (`TaskService`, `SyncStatusDao`, `ProjectDao`, …), so
after a restart the app shows CLI changes exactly as if they had been made in the app.

## Features

| Area | Commands |
| --- | --- |
| Tasks & notes | `list` (rich filters), `show`, `add`, `edit`, `complete`, `wontdo`, `reopen`, `delete`, `restore`, `move`, `duplicate` |
| Dates | all-day or timed, spans, durations, reminders, repeat rules (RRULE or `daily`, `every 2 weeks`, `every mon,fri`…), repeat from due or completion date |
| Repeating tasks | completing closes one occurrence (a completed copy is kept) and moves the task to its next date, like the app |
| Subtasks | `--parent`, `--no-parent`; completing a parent completes its subtasks, reopening a subtask reopens its parents |
| Checklists | `items list/add/check/uncheck/edit/delete/promote`, progress kept in sync |
| Projects | `list/show/add/edit/archive/unarchive/delete`, colours, list/kanban/timeline view, note lists |
| Folders | `folders list/add/rename/delete` |
| Kanban sections | `sections list/add/rename/delete`, `tasks edit --column` |
| Tags | `list/add/rename/merge/edit/delete`, nested tags; renaming rewrites every task |
| Habits | `list/show/add/edit/archive/delete`, `checkin` (yes/no and count habits), `fail`, `reset`, `note` |
| Focus | `pomos list` (records / daily summary), `pomos add`, `pomos delete` |
| Overview | `agenda`, `search`, `info`, `export` (everything as one JSON document), `comments`, `filters` |
| Low level | `sql` (read-only unless `--write`), `schema`, `backup` |
| Self-description | `guide` (agent manual), `commands` (command tree, `--json` for machines) |
| App control | `app status/stop/start/restart`, or `--restart-app` on any write |

## Install

Download a binary from the releases page, or build from source (Rust 1.85+):

```bash
cargo build --release                                          # native
cargo build --release --target x86_64-pc-windows-gnu           # Windows exe from Linux (needs mingw-w64)
```

To carry it into an airgapped network, build or download it outside and copy the single executable in. It has no runtime
dependencies, because SQLite is compiled in.

## Database location

`--db <path>`, else `$TICKTICK_DB`, else:

| Platform | Path |
| --- | --- |
| Windows | `%APPDATA%\Tick_Tick\TickTick.db` |
| macOS | `~/Library/Application Support/Tick_Tick/TickTick.db` |
| Linux | `~/.local/share/Tick_Tick/TickTick.db` |

## Writing safely

TickTick loads every task into memory at startup and writes whole rows back from that cache. Because of this:

* **Writes are refused while the app is running.** Close it with `ticktick-cli app stop`, or add `--restart-app` to the
  write. That closes the app, writes, and starts it again so the change is visible straight away.
  `--allow-running` overrides the check, but the app will not show the change until it restarts and may overwrite it.
* Before the first write of each day, the database is backed up to `TickTick.db.cli-backup-YYYYMMDD` next to it.
  `ticktick-cli backup` makes a backup on demand.
* Every command runs in one transaction. `--dry-run` runs it and rolls it back.
* Changes are queued for sync (`SyncStatusModel`, `sync_status`) the same way the app queues its own edits, so a copy
  that does reach TickTick's servers later uploads them normally.

`app start` finds `TickTick.exe` in the usual install folders. Otherwise use `--exe <path>` or set `$TICKTICK_EXE`.
On Linux it runs the exe through `wine`.

## Usage

```bash
ticktick-cli agenda                                  # overdue + next 7 days
ticktick-cli tasks list --due today
ticktick-cli tasks list -p Work -t urgent --priority medium --sort date
ticktick-cli tasks list --status completed --completed-from 2026-10-01
ticktick-cli tasks show 42                           # id, sid, sid prefix, or exact title

ticktick-cli --restart-app tasks add "Write report" -p Work --priority high -t work,urgent \
    -d "tomorrow 14:00" --duration 1h -r -15m -c "Q3 numbers"
ticktick-cli tasks add "Trip" -d 2026-11-02 --end 2026-11-05 -r "-1d@9:00"
ticktick-cli tasks add "Water plants" -d today --repeat "every 3 days"
ticktick-cli tasks add "Packing" -i socks -i passport
ticktick-cli tasks add "Sub step" --parent "Write report"

ticktick-cli tasks edit 42 --date "fri 10:00" --add-tag waiting --append "called Bob"
ticktick-cli tasks complete 42 43
ticktick-cli tasks move 42 -p Home
ticktick-cli tasks delete 42            # trash; --forever to delete
ticktick-cli items check Packing 1 passport --complete-task

ticktick-cli projects add Home --folder Life --color "#35D870"
ticktick-cli tags rename old new
ticktick-cli habits checkin Meditate
ticktick-cli habits checkin Water --value 2
ticktick-cli pomos add --start "today 9:00" --duration 25m -t 42

ticktick-cli --json tasks list | jq '.[].title'
ticktick-cli sql "SELECT title, startDate FROM TaskModel WHERE status = 0 LIMIT 5"
```

Run `ticktick-cli <command> --help` for every option.

### Date input

`today`, `tomorrow`, `yesterday`, weekday names (`fri`, `next monday`), `YYYY-MM-DD`, offsets (`+3d`, `-1w`, `+2h`,
`+30m`), each optionally followed by a time (`tomorrow 9:30`, `fri 5pm`, `2026-11-01 14:00`). A date with no time
makes an all-day task.

### Reminders

`0` (on time), `-5m`, `-1h`, `-1d`, `-1h30m` before the start; for all-day tasks `9:00` (on the day) or `-1d@9:00`
(the day before). Raw iCalendar values (`TRIGGER:-PT15M`) also work.

### Values

| | Stored | CLI |
| --- | --- | --- |
| Status | 0 / 2 / -1 | open / completed / wontdo |
| Priority | 0 / 1 / 3 / 5 | none / low / medium / high |

(The upstream tool showed priority 3 as "High". TickTick's high priority is 5.)

## For agents

The binary is self-describing: `ticktick-cli guide` prints the full agent manual (rules, id formats, input
syntax, JSON output shapes, recipes), and `ticktick-cli --json commands` prints the complete command tree with
argument types, defaults and allowed values. [AGENTS.md](AGENTS.md) explains how the writes are modelled.

## Development

```bash
cargo test   # unit tests + end-to-end tests against an empty 8.2.1.5 profile (tests/fixtures/schema.sql)
```

## License

MIT

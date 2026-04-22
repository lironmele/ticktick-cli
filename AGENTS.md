# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ticktick-cli is a read-only CLI tool that reads TickTick task manager data directly from the local SQLite database. No login or network access required.

## Build Commands

```bash
cargo build              # Debug build
cargo build --release    # Optimized release build
cargo run -- [args]      # Run without installing
```

Binary output: `target/release/ticktick-cli.exe` (Windows) or `target/release/ticktick-cli` (Unix)

## Architecture

Two source files only:

- `src/main.rs` - CLI argument parsing (clap), database connection, command dispatch
- `src/commands.rs` - All query/mutation logic and output formatting (tabled)

CLI follows `ticktick-cli <noun> <verb>` pattern: `projects`, `tasks`, `tags`, `pomos` as nouns; `list`, `show` as verbs.

Key data types: `ProjectRow`, `TagRow`, `TaskRowInternal`, `TaskDetail`. Query functions map database columns to these structs and render as ASCII tables.

## Database

Default locations:
- Windows: `%APPDATA%\Tick_Tick\TickTick.db`
- macOS: `~/Library/Application Support/Tick_Tick/TickTick.db`
- Linux: `~/.local/share/Tick_Tick/TickTick.db`

Key tables queried:
- `ProjectModel` - projects (columns: `_Id`, `id`, `name`, `color`, `sortOrder`, `isinbox`)
- `TaskModel` - tasks (columns: `_Id`, `projectId`, `title`, `content`, `status`, `priority`, `dueDate`, etc.)
- `TagModel` - tags (columns: `id`, `name`, `color`, `sortOrder`)

Task status values: 0=Pending, 2=Completed, -1=Cancelled. Priority: 3=High, 2=Medium, 1=Low, 0=None.


## Release Process

Tags starting with `v` trigger GitHub Actions workflow (`.github/workflows/release.yml`) that builds for Linux (x86_64), macOS (arm64), and Windows (x86_64) and creates a draft GitHub Release.

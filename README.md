# ticktick-cli

[中文](#中文) | [English](#english)

## 中文

滴答清单（TickTick）本地数据库 CLI 工具 - 只读版本。

直接从本地 SQLite 数据库读取滴答清单数据，无需登录，无网络依赖。

### 功能特性

- **列出项目** - 查看所有滴答清单项目
- **列出任务** - 支持按项目/标签筛选
- **查看任务** - 显示任务详情
- **列出标签** - 查看所有标签
- **番茄钟记录** - 查看专注记录，支持按日期/任务/项目筛选和按天汇总

### 数据库位置

默认读取以下路径的数据库文件：

| 平台    | 路径                                                  |
| ------- | ----------------------------------------------------- |
| Windows | `%APPDATA%\Tick_Tick\TickTick.db`                     |
| macOS   | `~/Library/Application Support/Tick_Tick/TickTick.db` |
| Linux   | `~/.local/share/Tick_Tick/TickTick.db`                |

### 安装

#### 从源码编译

```bash
cargo build --release
```

编译产物位于 `target/release/ticktick-cli`（或 `ticktick-cli.exe`）

#### 下载预编译二进制

从 [Releases](https://github.com/Lyt99/ticktick-cli/releases) 页面下载对应平台的二进制文件。

### 使用方法

命令格式：`ticktick-cli <noun> <verb>`

#### 项目 (projects)

```bash
# 列出所有项目
ticktick-cli projects list
```

#### 任务 (tasks)

```bash
# 列出所有待办任务
ticktick-cli tasks list

# 列出所有任务（含已完成和已取消）
ticktick-cli tasks list --show-all

# 按项目筛选
ticktick-cli tasks list --project inbox    # 收集箱
ticktick-cli tasks list --project 4        # 项目ID

# 按标签筛选
ticktick-cli tasks list --tags work

# 查看任务详情
ticktick-cli tasks show 10
```

#### 标签 (tags)

```bash
# 列出所有标签
ticktick-cli tags list
```

#### 番茄钟 (pomos)

```bash
# 查看最近番茄钟记录
ticktick-cli pomos list

# 查看指定日期的记录
ticktick-cli pomos list --date 2026-05-07

# 按日期范围查看
ticktick-cli pomos list --from 2026-05-01 --to 2026-05-07

# 按天汇总
ticktick-cli pomos list --from 2026-05-01 --to 2026-05-07 --summary

# 按任务或项目筛选
ticktick-cli pomos list --task 10
ticktick-cli pomos list --project inbox

# 限制显示条数
ticktick-cli pomos list --limit 20
```

#### 全局选项

```bash
# 指定数据库路径
ticktick-cli --db /path/to/TickTick.db tasks list
```

### 任务状态说明

| 状态      | 符号  | 说明   |
| --------- | ----- | ------ |
| Pending   | `[ ]` | 未完成 |
| Completed | `[x]` | 已完成 |
| Cancelled | `[-]` | 已取消 |

### 任务优先级

| 优先级 | 符号 |
| ------ | ---- |
| 高     | 🔴   |
| 中     | 🟡   |
| 低     | 🟢   |
| 无     | ⚪   |

### 构建要求

- Rust 1.75+

### 免责声明

本工具为只读工具，不会修改任何数据。使用前请自行备份数据库。

---

## English

TickTick CLI - A read-only CLI tool for accessing your local TickTick database.

Directly reads TickTick data from your local SQLite database. No login required, no network dependency.

### Features

- **List Projects** - View all TickTick projects
- **List Tasks** - Filter by project/tags
- **Show Task** - Display task details
- **List Tags** - View all tags
- **Pomodoro Records** - View focus records with date/task/project filters and daily summaries

### Database Location

Default database paths by platform:

| Platform | Path                                                  |
| -------- | ----------------------------------------------------- |
| Windows  | `%APPDATA%\Tick_Tick\TickTick.db`                     |
| macOS    | `~/Library/Application Support/Tick_Tick/TickTick.db` |
| Linux    | `~/.local/share/Tick_Tick/TickTick.db`                |

### Installation

#### Build from source

```bash
cargo build --release
```

Binary will be at `target/release/ticktick-cli` (or `ticktick-cli.exe`)

#### Download prebuilt binary

Get the binary for your platform from the [Releases](https://github.com/Lyt99/ticktick-cli/releases) page.

### Usage

Command format: `ticktick-cli <noun> <verb>`

#### Projects

```bash
# List all projects
ticktick-cli projects list
```

#### Tasks

```bash
# List pending tasks
ticktick-cli tasks list

# List all tasks (including completed and cancelled)
ticktick-cli tasks list --show-all

# Filter by project
ticktick-cli tasks list --project inbox
ticktick-cli tasks list --project 4

# Filter by tags
ticktick-cli tasks list --tags work

# Show task details
ticktick-cli tasks show 10
```

#### Tags

```bash
# List all tags
ticktick-cli tags list
```

#### Pomodoro

```bash
# Show recent pomodoro records
ticktick-cli pomos list

# Show records on a specific date
ticktick-cli pomos list --date 2026-05-07

# Show records in a date range
ticktick-cli pomos list --from 2026-05-01 --to 2026-05-07

# Show daily totals
ticktick-cli pomos list --from 2026-05-01 --to 2026-05-07 --summary

# Filter by task or project
ticktick-cli pomos list --task 10
ticktick-cli pomos list --project inbox

# Limit number of rows
ticktick-cli pomos list --limit 20
```

#### Global Options

```bash
# Specify database path
ticktick-cli --db /path/to/TickTick.db tasks list
```

### Task Status Legend

| Status    | Symbol | Description |
| --------- | ------ | ----------- |
| Pending   | `[ ]`  | Incomplete  |
| Completed | `[x]`  | Done        |
| Cancelled | `[-]`  | Cancelled   |

### Task Priority Legend

| Priority | Symbol |
| -------- | ------ |
| High     | 🔴     |
| Medium   | 🟡     |
| Low      | 🟢     |
| None     | ⚪      |

### Build Requirements

- Rust 1.75+

### Disclaimer

This tool is read-only and will not modify any data. Please backup your database before use.

---

## License

MIT

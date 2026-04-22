# ticktick-cli

[中文](#中文) | [English](#english)

## 中文

滴答清单（TickTick）本地数据库 CLI 工具 - 只读版本。

直接从本地 SQLite 数据库读取滴答清单数据，无需登录，无网络依赖。

### 功能特性

- **列出项目** - 查看所有滴答清单项目
- **列出任务** - 支持按项目/状态筛选
- **查看任务** - 显示任务详情
- **列出标签** - 查看所有标签

### 数据库位置

默认读取以下路径的数据库文件：

| 平台      | 路径                                                    |
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

```bash
# 查看帮助
ticktick --help

# 列出所有项目
ticktick projects

# 列出所有任务
ticktick tasks

# 仅显示未完成任务
ticktick tasks --pending

# 仅显示已完成任务
ticktick tasks --completed

# 查看指定项目的任务
ticktick tasks --project inbox    # 收集箱
ticktick tasks --project 4       # 项目ID（数字）

# 列出所有标签
ticktick tags

# 查看任务详情（使用数字ID）
ticktick show 10

# 指定数据库路径
ticktick --db /path/to/TickTick.db projects
```

### 任务状态说明

| 状态        | 符号    | 说明  |
| --------- | ----- | --- |
| Pending   | `[ ]` | 未完成 |
| Completed | `[x]` | 已完成 |
| Cancelled | `[-]` | 已取消 |

### 任务优先级

| 优先级 | 符号  |
| --- | --- |
| 高   | 🔴  |
| 中   | 🟡  |
| 低   | 🟢  |
| 无   | ⚪   |

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
- **List Tasks** - Filter by project/status
- **Show Task** - Display task details
- **List Tags** - View all tags

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

```bash
# Show help
ticktick --help

# List all projects
ticktick projects

# List all tasks
ticktick tasks

# Show only pending tasks
ticktick tasks --pending

# Show only completed tasks
ticktick tasks --completed

# Filter by project
ticktick tasks --project inbox    # Inbox
ticktick tasks --project 4       # Project ID (numeric)

# List all tags
ticktick tags

# Show task details (use numeric ID)
ticktick show 10

# Specify database path
ticktick --db /path/to/TickTick.db projects
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
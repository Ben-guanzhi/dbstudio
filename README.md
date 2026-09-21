# dbstudio

一个使用 Rust 与 [GPUI](https://gpui.rs) 构建的桌面数据库管理客户端，支持 SQLite、MySQL、PostgreSQL、SQL Server 和 Oracle。

## 功能

- **多数据库连接**：通过统一的连接抽象同时支持 5 种数据库，密码保存在操作系统 keyring 中
- **SSH 隧道**：为任何网络数据库建立本地端口转发（支持密码与密钥文件认证）
- **SQL 编辑器**：基于 tree-sitter 的语法高亮、SQL 自动补全与格式化
- **对象浏览**：数据库 / 模式 / 表 / 列 / 索引 / 外键树形浏览，可查看建表语句
- **结果面板**：表格化查询结果、数据编辑（INSERT / DELETE）、模式视图、CSV / JSON 导出
- **查询历史**：本地 SQLite 持久化保存查询历史与连接配置
- **多标签会话**：支持多连接标签页，右键菜单支持关闭/关闭其他/在新窗口打开
- **Vim 模式**：内置 Vim 键绑定支持，包括可视块选择和行操作

## 快捷键

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+Enter` | 执行 SQL 查询 |
| `Shift+Alt+F` | 格式化 SQL |
| `Ctrl+/` | 切换注释 |
| `Cmd/Ctrl+N` | 新建窗口 |
| `Cmd/Ctrl+K` | 打开命令面板 |
| `Cmd/Ctrl+P` | 快速打开 |
| `Shift+Cmd+D` | 复制行 |
| `Alt+↑/↓` | 上移/下移行 |
| `Ctrl+Z` | 撤销 |
| `Ctrl+Shift+Z` | 重做 |

### Vim 模式快捷键

| 模式 | 按键 | 功能 |
|------|------|------|
| Normal | `i/a/o` | 进入插入模式 |
| Normal | `dd` | 删除当前行 |
| Normal | `yy` | 复制行 |
| Normal | `p` | 粘贴 |
| Normal | `u` | 撤销 |
| Normal | `Ctrl+r` | 重做 |
| Visual | `v` | 字符选择 |
| Visual | `V` | 行选择 |
| Visual | `Ctrl+v` | 块选择 |

## 构建

需要稳定的 Rust 工具链（见 `rust-toolchain.toml`）：

```sh
cargo build --release
# 产物：target/release/dbstudio.exe
```

开发模式运行：

```sh
cargo run -p dbstudio
```

## 工作区结构

```
crates/
├── core      # 共享模型：ConnectionConfig、DatabaseType、SqlResult、schema 类型
├── db        # 数据库驱动层：各数据库连接实现 + SSH 隧道
├── storage   # 本地持久化：连接配置、查询历史（内置 SQLite，WAL 模式）
├── ui        # GPUI 组件：SQL 编辑器、表树、结果面板、连接表单等
└── app       # 可执行入口：窗口、主题、工作区装配
```

依赖方向：`app → ui → db → core`，`ui → storage → core`。

## 架构说明

### SSH 隧道

SSH 转发实现于 `crates/db/src/tunnel.rs`（基于 [`russh`](https://crates.io/crates/russh)）：

1. `SshTunnel::prepare` 校验连接配置中的 SSH 字段（缺失时报出可操作错误）
2. `SshTunnel::open` 连接 SSH 服务器（密码或密钥文件认证），在 `127.0.0.1:<临时端口>`
   上绑定本地监听，为每条连接打开 `direct-tcpip` 通道转发到数据库端点
3. `dbstudio_db::connect` 打开隧道后把配置的 host/port 覆写为回环端点，
   因此所有驱动通过 `Endpoint::resolve` 拿到的都是隧道地址，驱动本身对隧道无感知

注意：当前实现接受任意服务器主机密钥（TOFU / known_hosts 校验尚未实现），
且隧道随进程生命周期存活（每个连接一条隧道，不主动关闭）。

### 异步运行时边界

本项目刻意混用了两个异步运行时，边界如下：

| 运行时 | 使用范围 |
|---|---|
| **smol**（async-std 生态） | 应用主体：GPUI 事件循环、`dbstudio-storage`、`dbstudio-ui`、sqlx 的 MySQL/PostgreSQL/SQLite 连接 |
| **tokio**（专用后台线程上的独立 runtime） | 仅限要求 tokio 上下文的驱动：SSH 隧道（russh）、SQL Server（tiberius）、Oracle |

两个运行时之间唯一的交接点在 `crates/db/src/tunnel.rs`：smol 侧通过 tokio
oneshot channel 的 `blocking_recv` 等待隧道建立，之后转发任务完全运行在 tokio
runtime 线程上。修改代码时请遵守该边界——不要在 smol 执行器上阻塞等待 tokio
任务（反之亦然），也不要把 tokio runtime 泄漏到 `db` crate 之外。

### 数据兼容与命名空间迁移

当前版本统一使用 `dbstudio` 命名空间（`crates/core/src/lib.rs` 的 `NAMESPACE`）：

- 本地 SQLite 存储：`<local data>/dbstudio/dbstudio.db`
- keyring 服务名：`dbstudio`
- 主题配置：`<local config>/dbstudio/theme.txt`

从旧版（`dbclient` 命名空间）升级时会自动迁移，无需手动操作：

1. **SQLite 存储**：首次打开时，若新路径不存在且旧 `<local data>/dbclient/dbclient.db`
   存在，则整体复制到新路径（连接配置、查询历史、设置全部保留）
2. **keyring 密码**：读取连接密码时优先使用新服务名，找不到则回退读取旧
   `dbclient` 服务名的凭据并自动写入新服务名（惰性迁移）
3. **主题**：新命名空间没有主题记录时，复制旧 `theme.txt` 的选择

旧文件与旧 keyring 凭据**不会被删除**（作为备份，迁移失败可重试）；用户显式
删除连接或保存空密码时会同时清理新旧两处凭据。迁移逻辑分别位于
`crates/storage/src/store.rs`（文件迁移，含单元测试）、
`crates/storage/src/connections.rs`（keyring 回退）与 `crates/app/src/themes.rs`（主题）。

## 测试

```sh
cargo test --workspace
```

集成测试位于 `crates/db/tests/`：`sqlite_flow.rs`（SQLite 连接流程）与
`tunnel_config.rs`（SSH 配置校验、端点解析、隧道失败路径）。

## 许可证

暂未指定。

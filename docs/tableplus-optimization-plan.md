# dbstudio 优化方案（对标 TablePlus / TablePro）

> 基于 TablePlus 与开源兄弟项目 TablePro 的差异化优势 + dbstudio 现状的差距分析，产出分阶段可落地实施方案。
> 每个阶段独立可交付、可测试，阶段间有依赖顺序。
>
> **可行性前提**：已核实 `gpui-base`(0.6.1) 的 `EditorState` 暴露了
> `selected_range()/set_selected_range()/undo()/redo()/set_value()/set_highlighter()`
> 等 API，因此**选区执行、撤销/重做、格式化回填**均可行。
> 此外 `InputBaseState` 内置多光标动作与手势：Windows 下 `Ctrl-Alt-Up/Down`
> 添加光标上/下（macOS `Cmd-Alt`）、`Alt+Click` 追加光标、`Alt+Shift+Click` 列块，
> 故**多光标原生可用**；仅 `selections`/`CursorSelection` 为 `pub(super)`，
> 编程侧批量下光标不可用（不影响用户侧）。
> 当前 Vim 模式**已自研实现**（`crates/ui/src/components/vim.rs`，含 Visual 块
> 作为补充），AI 行内补全、Review 逐条 Apply、编辑器偏好持久化亦已落地。

---

## 差距总览对比表

| 能力 | TablePlus / TablePro | dbstudio 现状 | 优先级 | 可行性 |
|------|----------------------|----------------|--------|--------|
| 多标签 / 多窗口 / 分屏 | ✅ 核心 | ❌ 单连接单工作区 | P0 | ✅ 自研 |
| 数据网格：撤销/重做 | ✅ | ❌ | P1 | ✅ 借 `EditorState.undo/redo` |
| 数据网格：行内编辑 + Code Review | ✅ | ⚠️ 有编辑，无 diff 审查 | P1 | ✅ |
| Safe Mode / 生产库保护 | ✅（TablePlus） | ❌ | P1 | ✅ |
| 高级过滤器（结果集，按类型） | ✅ | ❌ 仅排序 | P1 | ✅ |
| SQL 编辑器：选区执行 / 格式化 / 收藏 | ✅ | ⚠️ 部分（格式化空按钮） | P2 | ✅ |
| 命令面板 / 快速跳转（Cmd-K） | ✅ | ❌ | P2 | ✅ |
| 查询历史全文搜索 | ✅ | ❌ 仅列表 | P2 | ✅ |
| 连接分组 / 标签（组织连接） | ✅ | ❌ | P2 | ✅ |
| SQL 编辑器：Vim 模式 / 多光标 | ✅（TablePro） | ⚠️ Vim 已实现；多光标原生可用 | P2 | ✅ Vim 自研完成；多光标原生 |
| AI：聊天 / 行内建议 / Explain/Optimize | ✅（TablePro） | ❌ | P2 | ⚠️ 需接 LLM API |
| MCP 服务器 / URL scheme | ✅（TablePro） | ❌ | P3 | ⚠️ 大工程，架构级 |
| 插件系统（用户自写驱动） | ✅（TablePro） | ❌（5 种内置） | P3 | ⚠️ 大工程 |
| 整库导出/导入（SQL Dump） | ✅ | ❌ 仅表内 CSV/JSON | P3 | ✅ |
| 原生快捷键全覆盖 | ✅ | ⚠️ 少量 | P2 | ✅ |

---

## 技术现状核查（gpui-base EditorState，决定可行性）

| API | 存在 | 用途 |
|-----|------|------|
| `selected_range()` / `set_selected_range()` | ✅ | 阶段 4a 选区执行 |
| `undo()` / `redo()` | ✅ | 阶段 2 数据网格撤销/重做 |
| `value()` / `set_value()` | ✅ | 阶段 4c 格式化回填、收藏加载 |
| `set_highlighter()` | ✅ | 语法主题 |
| Vim 模式 | ⚠️ 自研已完成 | `crates/ui/src/components/vim.rs`，Normal/Insert/VisualChar/VisualLine/VisualBlock，接线 + 单测 |
| 多光标 | ✅ 原生 | `InputBaseState` 内置：Ctrl-Alt-Up/Down 加光标、Alt+Click 追加、Alt+Shift+Click 列块；`selections` 为 `pub(super)`，编程侧不可批量设置 |
| AI 补全 | ❌ | 需接第三方 LLM 或 LSP |

---

## 阶段 1：多标签系统（P0，地基）

**目标**：每个连接一个独立 tab，同时打开多个连接，互不干扰。这是后续所有功能的基础。

### 状态改造

当前 `AppState` 是**单连接全局单例**。需引入 `ConnectionSession` 结构，把单连接状态装进去：

```rust
// crates/ui/src/state/session.rs
pub struct ConnectionSession {
    pub connection: Option<Arc<Connection>>,
    pub connection_state: ConnectionStatus,
    pub active_database: Option<String>,
    pub databases: Vec<DatabaseInfo>,
    pub tables: Vec<TableInfo>,
    pub table_schemas: HashMap<String, TableSchema>,
    pub last_result: Option<Arc<SqlResult>>,
    pub is_executing: bool,
    pub editor_buffer: Entity<EditorState>, // 每个 session 独立的编辑缓冲
}
```

- `AppState` 增加 `sessions: HashMap<SessionId, ConnectionSession>()` 与 `active_session: Option<SessionId>`。
- 连接时 `connect_async` 创建/激活一个 session，而非写入全局 `active_connection`。
- 所有现有操作（`execute_query`、`select_database`、`load_table_schema`）改为读取 `active_session`。

### UI 改造

新增 `TabsBar` 组件（`crates/ui/src/components/tabs.rs`）：

- 顶部水平标签栏，每个 tag = 一个连接（显示连接名 + 数据库 + 关闭按钮 + 连接状态圆点）。
- 点击切换 `active_session`；中键/按钮关闭（断开连接并释放 SSH 隧道 guard）。
- 标签支持拖拽排序（可选，P2）。

`Workspace` 渲染改为：活跃时 `render_tabs + render_active_session`，每个 session 有自己独立的 `Editor + ResultsPanel + TablesTree`。

### 涉及文件

- `crates/ui/src/state/mod.rs`（新增 session 集合）
- `crates/ui/src/state/operations.rs`（操作目标改为 active_session）
- `crates/ui/src/state/session.rs`（新文件）
- `crates/ui/src/components/tabs.rs`（新文件）
- `crates/app/src/workspace/mod.rs`、`workspace_view.rs`、`home_view.rs`

### 验收

- 同时连 MySQL + SQLite，各自独立执行查询，结果不互相污染。
- 关闭一个 tab 不影响其他 tab。
- SSH 隧道随 tab 关闭而释放。

---

## 阶段 2：Safe Mode + 变更审查 + 撤销/重做（P1）

**目标**：生产库保护 + 每次写操作前可见的 diff 审查，贴**近 TablePlus 的 code review 与 Safe Mode**，并在数据网格补齐 **TablePro 的撤销/重做**。

### 2a. 连接环境标记

- `ConnectionConfig` 增加 `environment: Environment`（`Dev | Staging | Production`），默认 `Dev`。
- 连接表单增加环境下拉框。
- storage schema 增列（`store.rs`），提供迁移（ALTER TABLE ADD COLUMN）。

### 2b. SQL 危险操作拦截（在 `execute_query` 前）

新增 `crates/ui/src/state/guard.rs`：

```rust
pub struct WriteGuard {
    pub sql: String,
    pub kind: WriteKind,        // UpdateNoWhere | DeleteNoWhere | Drop | Truncate | Ddl
    pub table: Option<String>,
    pub affected: Option<u64>,  // 若能通过 EXPLAIN 预估
}
```

- 用 `sqlparser`（或轻量正则）识别 SQL 首语句类型。
- 规则（`environment == Production` 时强制）：
  - `UPDATE ... 无 WHERE` → 弹确认。
  - `DELETE ... 无 WHERE` → 弹确认。
  - `DROP / TRUNCATE` → 弹确认。
  - 其他 DDL → 默认弹确认。
- `Dev/Staging` 环境可在 `safe_mode: bool` 开关开启同样拦截。

### 2c. 变更审查（Code Review）

把**数据编辑**从"即点即执行"改为"进待审队列"：

- 结果面板的 INSERT/UPDATE/DELETE 不再直接 `execute_query`，而是写入 `pending_writes: Vec<PendingWrite>`。
- 新增 `ReviewPanel`（在结果面板顶部或 footer）列出待执行的写操作：每条显示 **变更前→变更后的 diff**（列值对比）与生成的 SQL。
- "Apply All / Apply Selected / Discard" 按钮批量执行。
- 对应 TablePlus：用户始终掌控自己改了什么。

### 2d. 数据网格撤销/重做（TablePro 差异点）

- 结果面板的**单元格就地编辑**（双击改 `ResultCell`）进入网格级撤销栈。
- 复用 `gpui-base` 的 `EditorState.undo()/redo()`（已确认存在），或为 `ResultsTableDelegate` 维护独立 `Vec<PendingWrite>` 撤销栈。
- 网格编辑与 2c 的待审队列**共用** `PendingWrite`：每次格子改动入栈，`Ctrl-Z`/`Ctrl-Shift-Z` 撤销/重做该格的 old→new diff，Apply 时才批量写库。
- 表头附近加 undo/redo 按钮（禁用态随栈空/满）。

### 2e. 连接分组 / 标签（组织连接，呼应 TablePro iCloud 分组）

- `ConnectionConfig` 增加 `group: Option<String>` 与 `tags: Vec<String>`。
- `ConnectionList` 支持按 group 折叠分组显示，右键可打标。
- storage `connections` 表加 `group`、`tags`(JSON) 列并迁移（`store.rs`）。
- 命令面板可据标签过滤连接。

### 涉及文件

- `crates/core/src/models.rs`（`Environment` 枚举、`ConnectionConfig.environment/group/tags`）
- `crates/storage/src/store.rs`、`connections.rs`（schema + 序列化）
- `crates/ui/src/components/connection_form.rs`（环境字段）
- `crates/ui/src/state/guard.rs`（新）
- `crates/ui/src/state/operations.rs`（execute_query 接入 guard、pending_writes、撤销/重做操作）
- `crates/ui/src/components/results_panel/actions.rs`（编辑进队列而非直发）
- `crates/ui/src/components/results_panel/views.rs`（双击就地编辑）、`table_delegate.rs`（撤销栈）
- `crates/ui/src/components/review_panel.rs`（新）
- `crates/ui/src/components/connection_list.rs`（分组视图）
- `db/tests/`（SQL 分类单测）

### 验收

- Production 连接 + 无 WHERE DELETE 被拦截并弹确认。
- 编辑一行产生 diff，Apply 后才真正写库；`Ctrl-Z` 能回退。
- `cargo test --workspace` 全绿。

---

## 阶段 3：高级过滤器 + 命令面板 + 历史搜索（P1/P2）

### 3a. 结果集高级过滤器

TablePlus 可按列类型给过滤条件。在 `results_panel` 顶部加过滤条：

- 每列一行"过滤 chip"：`列名 + 操作符 + 值`。
- 按 `CellType` 提供不同操作符（`Text: = / contains / like`，`Integer/Float: = / > / < / between`，`Date/DateTime: after / before / between`）。
- 过滤作用于已加载的结果集（纯前端过滤，实时），或生成新的 `SELECT ... WHERE` 下拉加载（二选一，后者数据更大）。
- 先做**前端过滤**（无需重新查询），保留后续生成 SQL 的钩子。

### 3b. 命令面板（Quick Open / Cmd-K）

新增 `CommandPalette` 弹层组件，复用 GPUI 的 `input`：

- 输入即时匹配下列条目：
  - **表/视图**：跳转到某表（加载 schema + SELECT，复用现有 `build_select_query`）。
  - **数据库**：切换 active_database。
  - **连接**：切换 session / 新建连接（可按 2e 的标签/分组过滤）。
  - **命令**：格式化、执行、导出、切换主题、Safe Mode 开关。
- 键盘：`Cmd/Ctrl-K` 打开，方向键选择，回车执行，Esc 关闭。

### 3c. 查询历史全文搜索（呼应 TablePro 的 history search）

- `HistoryPanel` 顶部加搜索框；在内存 `query_history` 上按 SQL 子串（不区分大小写）过滤。
- 后续对大数据量历史再做 storage 侧的 `LIKE` 查询（`history.rs` 加 `search(sql_pattern)`）。
- 结果按时间倒序，可继续"重新执行"该查询（复用现有行为）。

### 涉及文件

- `crates/ui/src/components/results_panel/views.rs`（过滤条）
- `crates/ui/src/components/filter_bar.rs`（新）
- `crates/ui/src/components/command_palette.rs`（新）
- `crates/ui/src/components/history_panel.rs`（搜索框）
- `crates/storage/src/history.rs`（`search()`，可选）
- `crates/app/src/workspace/mod.rs`（绑定 Cmd-K，渲染 palette）

---

## 阶段 4：SQL 编辑器增强 + 轻量 AI（P2）

在 `sql_editor.rs` 逐项补齐：

### 4a. 选区执行
- 当前 `run_query` 传整段 `value()`。改为：有非空选区则执行选区（`state.selected_range()`，已确认存在），否则执行整段。

### 4b. 收藏查询（Favorites）
- storage 新增 `favorites` 表（`QueryHistoryRepository` 扩展或新 `favorites.rs`）。
- Editor 工具栏加 ★ 按钮收藏当前 SQL；侧边历史面板增加"Favorites"标签页。
- 复用现有 `QueryHistoryEntry` 或新增轻量结构。

### 4c. 格式化（补齐逻辑）
- 当前 format 按钮是空操作 `on_click(|_,_,_| {})`。
- `Cargo.toml` 已有 `sqlformat = "0.3"`。实现 `sqlformat::format(sql, options, Default::default())`，然后 `state.set_value()` 回填编辑器（确认可用）。

### 4d. 分屏（Split Panes）
- 编辑器支持垂直/水平分屏：`session.editor_buffers: Vec<Entity<EditorState>>` + split 视图。
- P2 可选，若工作量过大可降级为"结果面板与编辑器垂直分栏"。

### 4e. 流式结果（可选）
- `MAX_RESULT_ROWS = 10_000` 已截断。先做"加载更多"按钮（截断时追加 `LIMIT/OFFSET` 合并），低成本近似流式。

### 4f. 低成本编辑增强（Vim 已实现；多光标原生可用）
- **Vim 模式**（TablePro 差异点）**已自研完成**：`crates/ui/src/components/vim.rs` 纯逻辑 + 工具栏开关 + 持久化 + 11 项单测。
- **多光标**（TablePro 差异点）**原生可用**，无需自研：
  - `Ctrl-Alt-Up / Ctrl-Alt-Down`（macOS `Cmd-Alt`）添加光标到上/下一行；
  - `Alt+Click` 追加光标，`Alt+Shift+Click` 列/块选择（多行）；
  - 限制：`selections`/`CursorSelection` 为 `pub(super)`，编程侧无法批量写光标（用户侧不受影响）。
- 已落地：注释/取消注释 `Ctrl-/`、行重复/上移下移（`Shift-Alt-上下`）、括号自动闭合（gpui-base AutoClosingPair）、语法主题。

### 4g. AI 聊天 / 行内建议 / Explain（TablePro 差异点，可选）
- 采用**可插拔 Provider trait**（避免 vendor 锁定）：
  ```rust
  pub trait LlmProvider {
      async fn complete(&self, prompt: &str) -> anyhow::Result<String>;
  }
  // 实现：OpenAI 兼容 HTTP / Ollama（本地）
  ```
- 用**现有 `smol` + `reqwest`** 即可（不引入额外异步 runtime）。
- 功能位面：
  - 行内建议：光标处 `Tab` 触发补全（复用 `SqlCompletionProvider` 通道）。
  - Explain/Optimize：把当前 SQL 发给 LLM 返回优化建议，显示在侧边 `AiPanel`。
  - AI 聊天：`AiPanel` 里对话，附带上文中的 schema（表/列）作为上下文。
- `ConnectionConfig` 需能保存 provider/api_key（存 storage + keyring）；api_key 不落明文。
- 安全：AI 生成的 SQL 一律走 2b 的 WriteGuard，生产库默认只读。

### 涉及文件
- `crates/ui/src/components/sql_editor.rs`、`sql_completion.rs`
- `crates/storage/src/favorites.rs`（新）、`store.rs`
- `crates/ui/src/components/history_panel.rs`
- `crates/ui/src/components/ai_panel.rs`（新）、`crates/core/src/ai.rs`（LlmProvider trait，新）
- `crates/core/Cargo.toml`（`reqwest`，可选）

---

## 阶段 5：整库导出/导入（P3）

- `crates/db/src/export.rs`（新）：按数据库类型生成 SQL Dump。
  - MySQL：调用 `mysqldump`（子进程）或逐表 `SHOW CREATE TABLE` + `SELECT`。
  - SQLite：`.dump` / `VACUUM INTO`。
  - PG：`pg_dump` 子进程或逐表导出。
  - 复用现有 `get_create_table_sql` + 全表 SELECT 拼 DUMP。
- UI：结果面板/tables 树右键"Export Database…"，选路径写文件。
- Import 同理反向执行 dump。

---

## 阶段 6：MCP / URL scheme / 插件系统（P3，架构级大工程）

> 这三项是 TablePro 的差异化点，但都属于需要**重新设计进程/接口边界**的大工程，短期 ROI 低，列为远期方向，不进入近期迭代。

- **MCP 服务器**：dbstudio 作为 MCP host/server，让 Cursor/Raycast/Claude Desktop 通过数据库对话。需引入 `mcp` crate + JSON-RPC over stdio。
- **URL scheme**：`dbstudio://connect?config=...`，需在 `main.rs` 注册系统 scheme 处理。
- **插件系统**：仿 TablePro 用 Swift；Rust 侧可用 `wasmtime` 做 driver 插件沙箱，或用 `cdylib` + `dlopen` 动态加载。均需清晰插件 ABI，工程量大。

> 建议：若项目目标仅自用，跳过阶段 6；若愿景是"开源的 TablePro"，把插件系统提到优先级但接受大投入。

---

## 非功能建议（贯穿所有阶段）

1. **测试策略**：当前 UI 层无测试。为 `guard.rs`（SQL 分类）、`filter`（条件解析）、`export`（dump 生成）、`favorites`（CRUD）补充纯逻辑单测（`db/tests/`、`ui/src/state` 内 `#[cfg(test)]`）。
2. **键盘快捷键集中管理**：把散落的 `capture_action` / keybinding 收集到统一模块 `crates/app/src/keybindings.rs`，参考 TablePlus/TablePro 提供"默认快捷键表"。
3. **多窗口**：作为多标签的自然延伸，P3 后考虑 `App::new_window` 支持独立窗口（每个 window 一个 session 集合）。
4. **配置中心**：把 AI provider、Safe Mode 全局开关、主题、编辑器偏好统一收敛到 `AppState.settings`，通过 `keyring` + storage 持久化。

---

## 建议执行顺序

```
阶段1 多标签 ──► 阶段2 SafeMode+撤销+分组 ──► 阶段3 过滤器+命令面板+历史搜索
                                            │
阶段5 dump（P3，可穿插）◄── 阶段4 编辑器+轻量AI
阶段6 MCP/插件（远期，需单独立项）
```

**阶段 1 必须先行**（重构影响面最大，其他阶段都依赖 session 化）。阶段 2/3 相互独立，可并行。阶段 4 依赖阶段 2（编辑器撤销/编辑与数据网格共享机制）。阶段 5/6 与前者不冲突，按愿景投入度穿插。

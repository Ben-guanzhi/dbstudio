# dbstudio 界面重构计划（对标 TablePro）

> 参考 TablePro（Swift/AppKit，见 `D:\tools\db\TablePro-main`）的布局语言与交互细节，
> 用 gpui + gpui_component 重新设计 dbstudio 界面。**只借鉴设计，不移植代码**。
> 每阶段独立可交付、可测试（`cargo check/test/clippy` 全绿后提交）。

---

## 设计规范（从 TablePro 实测提取）

### 布局度量
| 区域 | 规格 |
|---|---|
| 标签栏带 | 高 36px，内嵌 28px 胶囊轨道（内边距 8、间距 4），标签高 24px，圆角全圆，min 宽 120px，`+` 新标签按钮 28×28 |
| 表格区顶栏（编辑器命令条） | 横向 padding 12 / 纵向 6，间距 8，控件 small 尺寸 |
| 网格表头 | 高 28px；排序箭头小号、多列排序显示优先级数字；筛选漏斗图标 |
| 网格行高 | 紧凑 20 / 正常 24 / 宽松 28 / 宽松+ 32，默认 24；**交替行条纹默认开** |
| 底部状态栏 | 高 28px，横向 padding 10，簇间距 8，顶部分隔线 |
| 字体 | 编辑器/网格等宽 13px；标签文字 11px；状态栏 caption |
| 分隔线 | 1px hairline |

### 颜色语义（TablePro 主题 JSON 实测值，映射到 theme token）
| 语义 | 值 |
|---|---|
| success | `#248A3D`，warning `#C55B00`，error `#D70015`，accent `#007AFF` |
| badge 背景 | `#E5E5EA`（PK 蓝 15%、自增紫 15%） |
| 网格 diff 着色（30% alpha） | 修改 `#FFD60A`、插入 `#34C759`、删除 `#FF3B30` |
| NULL 单元格 | 次要文字色 + 斜体，字面 `NULL` |
| 行号 gutter | 次要文字色 |
| 环境徽章 | Dev/Local 绿底、Staging 橙底、Production 红底 |

### 顶栏信息胶囊（TablePro 标题带中央）
`[环境徽章] 服务器类型+版本 | [数据库图标] 当前库名 | 锁(SSL) ~延迟`

### 图标映射（SF Symbols → gpui_component IconName，取可用者）
Run `play.fill`→Play/Thunderbolt、Stop→Stop/Close、Refresh→RotateCw/Refresh、
History→History/Clock、Search→Search、Add→Plus、Close tab→X、Filters→Filter、
Columns→Eye、Export→Upload、Import→Download、Favorite→Star、Sidebar→PanelLeft、
DB `cylinder`→Database、Connection `network`→Globe、Error→TriangleAlert、Format→AlignLeft。

---

## 阶段 A：标签栏 TabsBar（P0，TablePro 核心交互）

**目标**：一个窗口内多个会话标签（TablePro 的 editor tab strip），打通已有 session 目录。

- 新组件 `crates/ui/src/components/tabs.rs`：
  - 36px 带 + 28px 胶囊轨道；每项：状态圆点（connecting 橙/connected 绿/disconnected 灰）
    + 会话名（连接名·库）+ 关闭 `X`（hover 显示）；选中=实心胶囊，未选=透明；
    `+` 按钮 → 打开连接表单（inline form），连接成功后落为新标签。
  - 溢出横向滚动；点击= `switch_session(id, window_id, cx)`；关闭= `close_session(id, cx)`
    （已有关闭逻辑会重排各窗口指针）。
- `Workspace`：`tabs: Entity<TabsBar>` 插到 header 之下；`observe_global` 同步
  `state.sessions` + `window_state.active_session`；编辑器缓冲已由
  `sync_editor_buffer` 按 session 切换，天然工作。
- 键位：`ctrl-tab`/`ctrl-shift-tab` 循环窗口内标签（可选，若键位冲突则跳过）。
- 测试：TabsBar 无逻辑则不加；`switch_session`/`close_session` 已有覆盖。

## 阶段 B：顶栏与主布局精修

**目标**：向 TablePro 标题带/信息架构看齐。

- `header_bar.rs` 重做：
  - 左：sidebar 折叠按钮（Tables）、History 折叠按钮、Favorites 切换（跳 History 面板 Favorites tab）；
  - 中：信息胶囊 `[环境徽章] 连接名 服务器版本 | 数据库 | safe-mode 锁`
    （数据来自 active session：`Environment`、connection name、`active_database_for`；
    服务器版本暂缺则省略该段，不做驱动层改动）；
  - 右：Search（开命令面板）、New Window、New Tab(`+`)、AI 面板、主题切换。
- Home 视图微调：连接卡片显示 group/tags 徽章（badge 样式 `#E5E5EA`），空态提示
  （`tray` 空态文案）。
- 保持三栏 resizable 结构不动（分屏阶段 4d 仍不做）。

## 阶段 C：数据网格 + 编辑器体验

- 网格（`results_panel/table_delegate.rs`/`views.rs`）：
  - 交替行条纹（默认开）；行高常量集中定义（24 默认）；NULL 灰色斜体；
  - 选中行整行 accent 强调（现有选中样式核对）；
  - pending-edit diff 着色：修改黄 30%、插入绿 30%、删除红 30%（行级）；
  - 表头排序指示 + 已有筛选漏斗保持。
- 编辑器命令条按 TablePro 顺序重排：
  `[数据库选择] …spacer… [Format][★收藏][Explain→AI][Run▾/Stop]`，图标化、small 尺寸。
- 状态栏（`footer_bar.rs`）重排为 TablePro 结果栏：
  左：`N rows [+] · status_message · 耗时`；右：safe-mode 锁徽章、vim 状态、
  Filters/Columns 切换（接线到结果面板已有能力，若无对应面板动作则仅保留已有按钮）。

## 阶段 D：收尾优化

- 历史搜索下沉到 storage：`history.rs` 加 `search(pattern) -> LIKE`（TablePro 是 FTS5，
  dbstudio 用 LIKE 足够）；面板改为按需全量查询（数据量小，可仍前端过滤 + 保留 TODO）。
- README 增补：多窗口/新标签/快捷键表。
- 计划文档更新状态标记。

---

## 执行顺序与验收

```
A TabsBar ──► B 顶栏/布局 ──► C 网格/编辑器 ──► D 收尾
```

- 每阶段：`cargo check --workspace` → `cargo test --workspace` → `cargo clippy`
 （新代码零警告）→ `rustfmt` 改动文件 → 单独 commit。
- 验收 A：一窗口内 MySQL+SQLite 两标签并存，切换时编辑器/结果/树各自独立，
  关一个不影响另一个。
- 验收 B/C：布局度量对齐上表；NULL/条纹/diff 着色肉眼可辨；顶栏胶囊随切换更新。

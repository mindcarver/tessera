# Epic 1 Context: 本机 Codex 记忆发现与搜索（Foundation + 首个端到端闭环）

<!-- Generated from planning artifacts. Regenerate with compile-epic-context if planning docs change. -->

## Goal

让 Carver 在本机启动 Tessera，发现并逐一确认 Codex Agent Memory 来源，建立只读 Derived Index，按关键词搜索，并从结果回到原始记忆位置，形成单一 Provider 的“看见 + 找到 + 打开”闭环。它同时固化后续 Provider 接入、浏览、韧性与项目联邦共用的本地优先边界、来源身份、索引和查询契约。

## Stories

- Story 1.1: 本地应用骨架与可启动运行
- Story 1.2: Codex Candidate Source 自动发现与展示
- Story 1.3: Source 确认、拒绝、停用与持久身份
- Story 1.4: 只读扫描管线与原子代际切换
- Story 1.5: Codex 记忆解析、边界限定与 canonical 记录
- Story 1.6: 关键词搜索与 Provenance 结果展示
- Story 1.7: 从结果打开原始记忆位置
- Story 1.8: Source Inventory、健康状态与手动重扫
- Story 1.9: Phase 0 性能基准门禁

## Requirements & Constraints

- 发现 Candidate Source 时展示 Provider、路径、依据与可判定 Native Project，但不读聊天或 transcript。仅 Confirmed Source 可读；确认、拒绝、停用须持久化，且不改写源数据。Native Project 保留原生身份，无法确认则显示“未映射”；无候选时不提供任意目录手动添加。
- 只索引 Codex 自动生成的 Markdown：`MEMORY.md`、`memory_summary.md`、`raw_memories.md`、`rollout_summaries/*.md`。排除 rollout/transcript JSONL、session、状态库对话、人工规则与 root 外内容；未知工件记录诊断但不入索引。
- 搜索已索引的 Confirmed Source，展示原始片段和完整 Provenance；空结果区分无匹配、未索引、不可用。Inventory 诚实显示范围、覆盖、健康、最近扫描、记录数和安全错误；重扫只作用于指定 Source，Health 至少覆盖路径、权限、格式和扫描问题。
- MVP 单用户、离线、无账号、无出站网络，日志不含正文、查询词或凭据。每次访问均重验 allowlisted root，不可信内容不得执行。扫描只在完整成功时切换，失败保留上一成功索引且查询继续可用。
- Codex adapter 须通过 fixture、零源写入、parser 版本、reconcile 恢复与能力诚实测试。Phase 0 用真实匿名 fixture 建立 cold scan、query、memory、index-size 基准；阈值验证前不得编造。

## Technical Decisions

- 采用 local-first 六边形模块化单体。Rust core 是唯一文件、业务和 SQLite 边界；React UI 仅调用版本化 HTTP API。Tessera 投影、Registry 和状态位于 OS-managed app-data，Source 在应用外且只读。
- 本地 Web 由 Rust core 内嵌同步 HTTP 服务和系统浏览器 UI 组成：仅绑定 `127.0.0.1`，校验 Host/Origin，返回收紧 CSP。请求与 SSE 带 `api_version`；查询使用受限 `cursor + limit`，扫描以单调 sequence 的 SSE 报告并支持取消；无外网、WebSocket 或远程访问面。
- 栈为 Rust stable 1.97.x、同步 HTTP crate、React 19.2.7、Vite 8.1.x、`rusqlite` 0.40.1（`bundled`）、SQLite FTS5、`notify` 8.2.x、`dirs` 6.x；精确 patch 由 lockfile 与 toolchain 文件锁定。核心按 domain、application、adapters、index、state、policy、http 分层。Phase 0 先验证 FTS5 中文查询、不可信 Markdown 的 CSP/sanitizer 和外部 SQLite 只读/WAL 行为，再确定解析与查询路径。
- Adapter 声明 discover、enumerate、search、watch、stable native IDs 和 coverage。`full`、`search_only`、`existence_only`、`unsupported` 不能混用语义。确认时 canonicalize root 并分配持久 `source_id` 与版本化 fingerprint；后续操作仅接收 `source_id` 或 `record_id`。
- Canonical record 保留 provider、来源、native identity、locator、revision、parser version、coverage 与观察时间；身份由 source、provider、native locator、unit kind 稳定生成，content hash 仅检测变化。无法稳定拆分 Markdown 时用 file-level unit，行范围只用于展示和打开。
- SQLite/FTS5 是可重建投影。每个 Source 串行写 staging generation，携带持久 scan state 与 fencing token，在同一事务 CAS 激活；最终 manifest 不一致则标记 `dirty_after_validation` 并重试。生命周期、Health、Coverage、scan state 与 active generation 分开建模。打开原始位置和错误处理均由 core 再验 allowlist，并使用不泄露正文的结构化错误信封。

## UX & Interaction Patterns

当前没有独立 UX 设计契约；仅落实候选依据与逐项确认、无候选空态、Inventory 状态、带 Provenance 的结果卡片、打开失败反馈及三种搜索空态。发现、搜索、筛选、Inventory 与打开均须键盘可达，具有可读状态、稳定 focus order 和 EmptyState；视觉细节待后续 UX 决策补充。

## Cross-Story Dependencies

Story 1.1 提供应用、迁移、测试与基线骨架。发现/确认供扫描、解析、Inventory 和重扫共用；扫描和 canonicalization 供搜索、Provenance 与打开共用；性能门禁依赖扫描和搜索。本 Epic 固化的 Adapter、Registry、代际切换、Query Service 与 HTTP 契约供后续 Epic 复用。

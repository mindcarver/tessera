# Epic 1 Context: 本机 Codex 记忆发现与搜索（Foundation + 首个端到端闭环）

<!-- Generated from planning artifacts. Regenerate with compile-epic-context if planning docs change. -->

## Goal

让 Carver 在本机启动 Tessera，自动发现 Codex 的 Agent Memory 来源，逐个确认后建立只读 Derived Index，按关键词搜索，并从结果卡片追溯到原始记忆位置 —— 端到端跑通单 Provider 的"看见 + 找到 + 打开"闭环。本 Epic 同时是 Foundation：锁定 Phase 0 脚手架与技术栈、固化 Rust core 唯一边界与 ProviderAdapter 契约，为后续 Claude Code 接入（Epic 2）、浏览（Epic 3）、韧性（Epic 4）、项目联邦（Epic 5）奠定风险已降级的基础。

## Stories

- Story 1.1: 本地应用骨架与可启动运行（Phase 0 脚手架）
- Story 1.2: Codex Candidate Source 自动发现与展示
- Story 1.3: Source 确认/拒绝/停用与持久身份
- Story 1.4: 只读扫描管线与原子代际切换（骨架）
- Story 1.5: Codex 记忆解析、边界限定与 canonical 记录
- Story 1.6: 关键词搜索与 Provenance 结果展示
- Story 1.7: 从结果打开原始记忆位置
- Story 1.8: Source Inventory、健康状态与手动重扫
- Story 1.9: Phase 0 性能基准门禁

## Requirements & Constraints

**功能边界（本 Epic 覆盖）：** 自动发现 Codex Candidate Source（含 Provider、候选路径、发现依据、可判定 Native Project，发现阶段不读聊天）；逐个确认/拒绝/停用 Source，确认状态重启保留；Source Inventory 展示单行 Codex 的 Coverage/Health/记录数/最近错误；原样保留 Native Project，无法确认显示"未映射"；限定 Codex Agent Memory 边界（排除聊天/transcript/规则文件，标明 Provider Memory 类型）；以只读方式建立 Derived Index（源文件集合/内容/大小/mtime 不变）；关键词搜索 Confirmed Source；展示原始片段 + 完整 Provenance；从结果打开原始位置；本机启动无账号、断网可用、重启保留。手动重扫（FR-8 子集）与 Codex Health（FR-13 子集）在此引入。

**非功能约束：** Agent Memory 以 Confirmed Source 为事实源，Derived Index 仅可重建视图；正常运行不得上传 Agent Memory、查询词、映射或诊断数据；默认日志不含正文/查询词/凭据；Tessera 只读取用户确认范围，UI 不暴露任意文件读取；路径/symlink/权限/身份变化必须重新通过边界校验；Agent Memory 按不可信内容渲染，不执行 HTML/脚本/命令；扫描只有完整成功才切换可见 generation，失败保留上一成功版本；Tessera 自有索引损坏可仅依赖 Confirmed Source 完整重建；扫描不应阻断对上一成功索引的查询；发现/搜索/筛选/打开核心操作必须键盘可达。

**质量门禁：** Phase 0 必须用 Carver 真实数据建立 cold scan / query / memory / index-size 基准并锁定为回归门禁，基准完成前不编造固定阈值；Codex adapter 五类契约测试（fixture contract / zero-source-mutation / parser-version / reconcile-recovery / capability-honesty）全部通过后才能在默认构建启用。

**明确反目标：** MVP 不提供任意目录手动添加（无候选时不显示入口）；不索引聊天、session transcript、人工指令或项目规则；不做 AI 摘要、嵌入、向量搜索、语义推理或 MCP/HTTP 服务；Connector 数量、记忆条数、AI 回答流畅度不是优化目标，来源可追溯与源保真才是。

## Technical Decisions

**栈与结构种子（Phase 0 锁定）：** Tauri 2.x + Rust（stable 1.97.x，精确 patch 写入 `rust-toolchain.toml`）+ React 19.2.7 + Vite 8.1.x + rusqlite 0.40.1（`bundled` feature）+ SQLite 3.x（FTS5 enabled）+ notify 8.2.x。lockfile 与 toolchain 文件在 bootstrap 时即拥有精确 patch。Rust core 模块骨架：`src-tauri/src/{domain, application, adapters, index, state, policy, ipc}`；UI 模块骨架：`src/{features, components, ipc}`。固定产出路径：`tests/ui/accessibility.spec.ts`、`tests/benchmarks/memory-index.json`、Provider fixture `src-tauri/tests/fixtures/providers/{codex,claude_code}`。

**Rust core 是唯一应用边界：** 所有文件访问、Provider 解析、索引写入、项目映射和查询协调必须经 Rust core application service；UI 只能调用已登记的 typed Tauri command；UI 不直接依赖 Provider、文件系统或 SQLite。discover 只产 Candidate 元数据；确认后 core canonicalize root 并保存 allowlist；后续命令只接受 `source_id`/`record_id`，不接受任意路径/SQL/句柄；每次读取重新校验目标仍在 root 内。

**ProviderAdapter 契约：** 每个 Adapter 声明 `discover`/`enumerate`/`search`/`watch`/`stable_native_ids`/`coverage_level`；契约固定在 `src-tauri/src/domain/ports/provider_adapter.rs`。Adapter 输出归一化 canonical envelope：`unit_kind`、`native_unit_id`、normalized `native_locator`、title/body、scope、`source_revision`、`parser_version`。Coverage Level 取值 `full | search_only | existence_only | unsupported`，`search_only/existence_only/unsupported` 不得展示为"完整同步"。

**Codex 数据边界（本 Epic 唯一 Provider）：** 纳入 `~/.codex/memories` 或 `CODEX_HOME/memories` 下自动生成的 Markdown：`MEMORY.md`、`memory_summary.md`、`raw_memories.md`、`rollout_summaries/*.md`。排除 rollout/transcript JSONL、session 内容、状态库对话内容、root 外目录。未知文件记 `unsupported_artifact` 诊断但不索引；文件集随 Codex 版本变化，须用 fixture 验证并记录 `parser_version`，格式漂移时显示 degraded 而非静默漏读。

**Source 身份与持久化：** Source 确认时分配持久 `source_id`；re-discovery 按 `provider + canonical root fingerprint` 匹配。Fingerprint 格式 `root-fingerprint/v1`，由 provider + root kind + normalized root path + filesystem identity `(device, file_id)` 构成；identity 不可用时以 normalized path 作显式 fallback。路径变化保留旧 Source 为 degraded 并产生新 Candidate，不自动合并；歧义/碰撞保持独立 Candidate，须显式 rebind。Source Health 变化不删除确认关系。

**Canonical 记录身份：** `record_id` 由 `source_id + provider + native locator + unit kind` 稳定生成；locator-based 而非 parser/content-based。`native_unit_id` = provider id / heading path + duplicate ordinal / file-level fallback；无法稳定拆分按 file-level unit，不宣称 section identity。File line range 仅用于打开/展示，不参与身份。Content hash 只用于变化检测；parser version 只作解析版本与重建触发。

**扫描代际与原子切换：** 每个 Source 由单一 Scan/Reconcile owner 排队处理；扫描先写 staging generation，只有完整成功才在一次事务中 CAS 切换 active generation。Scan/reconcile 持持久单调 fencing token + generation intent；取消/超时/retry/crash 恢复后旧 owner 不得 commit；commit 在同一事务 compare-and-swap（token + intent），仅 CAS 成功才切 active。`scan_runs` 持久化 `queued/running/staging/committing/succeeded/failed/retry`，进程启动回收 stale run。一致性级别 `snapshot-at-validation`；commit 前最终 fence/manifest 校验（size/mtime/hash + parser version）；验证后或 commit 中检测到 mutation 标记 `dirty_after_validation`，永不激活，调度有限 retry。

**Derived Index 与迁移：** Tessera SQLite、Source Registry 状态、项目映射属 Tessera 自有数据，可删除可重建，禁止回写 Source。Reset Index 清理 canonical body、FTS、scan runs 但保留 Source Registry 与 Tessera Project mappings；移除 Source 清理其派生 records。migration 原子执行，失败保留旧 index（migration 框架在 Phase 0 就绪，v0 起步）。

**IPC 与版本化：** 请求-响应用带 `api_version` 的 typed Tauri Commands；查询统一 `cursor + limit`（server-side bound）；低频状态用 Events；扫描进度用带递增 sequence 的 Channels 并支持 cancellation token；不开放 localhost HTTP/WebSocket/远程 URL。Cursor 携带 generation、projection revisions、sort key、record_id；snapshot token 绑定 active generation + project_mapping_revision + filter/policy revision + sort key；任一 revision 变化返回 `stale_snapshot`。

**结构化错误信封：** core 拥有共享 error envelope（stable `code` + safe `message` + `source_id` + phase）；Source 失败从不影响无关 Source generation；错误展示不含正文/凭据。

**Local-only 强制：** MVP 无出站网络路径；日志 omit body/query/credential；Phase A 仅支持 Carver 当前本机单一 Tauri 进程；Source roots 只读且位于应用外部；Tessera index/config/scan state 位于 OS-managed app-data。

**Phase 0 待验证项（不得用未验证便利实现绕过）：** FTS5 中文 tokenizer（`trigram` vs `unicode61` 在真实样本上的召回/MRR/空结果率/延迟/索引体积）、Markdown 与 Agent Memory 不可信内容的 CSP/sanitizer 方案（`default-src 'self'`、禁远程脚本、禁 raw HTML/script/event handler/javascript URL）、外部 SQLite `mode=ro`/WAL sidecar 可行性（禁 `immutable=1` 与 `nolock=1`）、exact toolchain build check（`rust-toolchain`/Tauri build/bundled FTS5/Capability smoke/installer launch/WebDriver smoke）。结论由 Phase 0/security-test owner 验证后决定是否提升为新 AD，并作为 Story 1.5/1.6 实现路径依据。

## UX & Interaction Patterns

无独立 UX 设计契约；UX 决策以"待定 UX 决策"或 dev 阶段决策形式嵌入对应 Story，AC 标注可验证的功能性约束。Epic 1 涉及的推迟决策：Candidate Source 展示与逐个确认/拒绝交互（无候选空态、无手动添加入口）；Source Inventory 状态卡片结构化展示（Coverage/Health/最近扫描/记录数/最近错误，记录数仅在能完整枚举时显示完整）；搜索关键词输入与组合筛选范围可见性（当前 Epic 范围=单 Codex Source）；空结果三态区分（无匹配/未索引/不可用）；结果卡片原始片段 + 完整 Provenance + Coverage/Health 标注；打开原始位置（只打开不编辑，失效时展示可理解错误 + Health）；键盘可达性（核心路径键盘完成、共享 focus order、可读状态标签、EmptyState，验收产物 `tests/ui/accessibility.spec.ts`）。视觉/交互细则待 UX 阶段补强，须先写成可测试的交互契约再实现视觉。

## Cross-Story Dependencies

Epic 1 是基础 Epic，无前置依赖。Story 1.1（Phase 0 脚手架 + Deferred 验证）是其他所有 Story 的基础，其产出的模块骨架、IPC 框架、migration 框架、fixture 目录、基准与可访问性占位文件被 1.2–1.9 复用。Story 1.2→1.3（发现→确认）共享 Source Registry；Story 1.3→1.4（确认→扫描）以 Confirmed Source 为扫描前提；Story 1.4→1.5（代际管线→Codex 解析）共用 staging generation 与 canonical envelope；Story 1.5→1.6（canonical 记录→搜索）以 Derived Index 为查询源；Story 1.6→1.7（结果→打开）共享 Provenance locator；Story 1.8（Inventory + 手动重扫）复用 1.3–1.5 的确认与扫描机制；Story 1.9（基准门禁）依赖 1.4/1.5 的扫描与 1.6 的搜索已可用，产出被后续所有 Epic 共享。本 Epic 固化的 ProviderAdapter 契约、Source Registry、Query Service、IPC 与代际切换机制是 Epic 2–5 的直接前置依赖。

---
stepsCompleted:
  - step-01-validate-prerequisites
  - step-02-design-epics
  - step-03-create-stories
  - step-04-final-validation
inputDocuments:
  - prds/prd-tessera-2026-07-20/prd.md
  - architecture/architecture-tessera-2026-07-20/ARCHITECTURE-SPINE.md
  - prds/prd-tessera-2026-07-20/addendum.md
  - ../specs/spec-tessera/requirements-matrix.md
  - ../specs/spec-tessera/SPEC.md
---

# Tessera - Epic Breakdown

## Overview

This document provides the complete epic and story breakdown for Tessera, decomposing the requirements from the PRD, UX Design if it exists, and Architecture requirements into implementable stories.

Tessera 第一阶段产品：一个供个人开发者在本机使用的、**只读**的跨 Agent 记忆资产浏览器。原生 Agent Memory（Codex、Claude Code）留在原位，Tessera 只生成可删除可重建的 Derived Index，持续只读联邦地清点、搜索、浏览并追溯来源。不迁移、不接管、不重写各 Agent 的原生记忆。

## Requirements Inventory

### Functional Requirements

- **FR-1 自动发现 Candidate Source：** 启动后自动发现本机受支持的 Codex 与 Claude Code Candidate Source；结果标明 Provider、候选路径、发现依据和可判定 Native Project；发现阶段不读取聊天记录；无结果时不显示手动添加目录入口。
- **FR-2 确认或拒绝 Source：** 用户可逐个确认/拒绝 Candidate Source；只有 Confirmed Source 进入正文扫描与索引；未确认/被拒绝来源不进 Derived Index；可停用已确认 Source 且不修改原始记忆；确认记录重启后保留。
- **FR-3 查看 Source Inventory：** 显示每个 Source 的 Provider、路径、Native Project、Coverage Level、Source Health、最近成功扫描时间、记录数量和最近错误；数量仅在能完整枚举时展示为完整；`search_only`/`existence_only`/`unsupported` 不展示为"完整同步"；Health 变化不删除确认关系。
- **FR-4 保留 Native Project：** 按 Provider 原样保留每条 Agent Memory 的 Native Project；无法确认的映射显示为未映射而非猜测归类；同一 Native Project 下记忆可独立搜索。
- **FR-5 建立 Tessera Project 映射：** 可创建 Tessera Project 并将 Codex/Claude Code 的一个或多个 Native Project 关联到同一 Tessera Project；映射仅存在于本地状态，不改 Provider 目录/文件；可查看/调整/移除映射且不删除源数据或索引记录。
- **FR-6 限定 Agent Memory 边界：** Connector 只纳入 Provider 自动生成的 Agent Memory；原始聊天、session transcript、完整对话、`CLAUDE.md`/`AGENTS.md`/项目规则不进 Derived Index；每条记录标明 Provider Memory 类型，不靠正文猜测类型。
- **FR-7 以只读方式建立 Derived Index：** 从 Confirmed Source 建立 Derived Index，扫描/重建过程保持原始 Agent Memory 不变（文件集合/内容/大小/修改时间不变）；删除索引后可重建；失败扫描不用不完整结果替换上一成功版本。
- **FR-8 更新 Derived Index：** 检测 Confirmed Source 变化并更新索引；用户可手动触发指定 Source 重新扫描；新增/修改/删除在成功扫描后反映到查询；扫描过程与最终状态对用户可见；手动扫描只作用于指定 Confirmed Source。
- **FR-9 搜索 Confirmed Source：** 输入关键词在全部或指定 Confirmed Source 的 Derived Index 查询；默认搜索所有健康且成功索引的 Source；不调用外部模型或远程搜索；空结果区分"确实无匹配""Source 未索引""Source 当前不可用"。
- **FR-10 筛选搜索结果：** 按 Provider、Confirmed Source、Tessera Project、Native Project、Agent Memory 类型和时间筛选；组合筛选时界面显示当前生效范围；清除筛选恢复全部 Confirmed Source 范围。
- **FR-11 展示原始结果与 Provenance：** 每条结果展示原始 Agent Memory 片段及完整 Provenance（Provider、Source、Native Project、原始文件或 Provider 引用、定位信息、来源更新时间）；显示 Coverage Level 与 Source Health；不把推断标题或项目映射伪装成 Provider 原始事实。
- **FR-12 打开原始位置：** 可从结果卡片打开或定位 Provenance 指向的原始 Agent Memory；只打开/定位不在应用内编辑；原始位置失效时展示可理解错误和 Source Health 状态。
- **FR-13 展示 Source Health：** 把每个 Confirmed Source 标记为 `unknown`/`healthy`/`degraded`/`error` 并给出可理解原因；状态至少区分路径失效、权限不足、格式不支持、扫描失败；错误展示不含 Agent Memory 正文或凭据。
- **FR-14 隔离 Connector 失败：** 一个 Connector/Confirmed Source 失败时其他 Source 仍可搜索；单个失败不导致全局搜索不可用；失败 Source 的上一成功结果若继续展示须标明上次成功时间和 stale 状态。
- **FR-15 重建 Derived Index：** 可删除并完整重建 Derived Index 而不影响 Confirmed Source 和 Tessera Project 映射；重建前明确告知只会删除 Tessera 派生数据；重建后恢复相同来源记录的稳定身份和 Provenance；重建失败时原始 Agent Memory 保持不变。
- **FR-16 浏览 Agent Memory 集合：** 可从 Source Inventory 或 Tessera Project 进入记忆集合，查看分页列表、最近变化和按条件筛选的 Agent Memory；浏览与搜索共用同一 Provenance、Coverage Level 和 Source Health 字段；空集合区分"尚未扫描""无可索引 Agent Memory""Source 当前不可用"；列表不含原始聊天、人工指令文件或未确认 Source。
- **FR-17 可视化记忆结构：** 可通过列表、分组和状态视图理解各 Provider、Tessera Project、Native Project 和 Agent Memory 类型之间的关系；可从 Provider 进入项目再进入记忆条目和原始位置；视图显示最近扫描、最近变化和 Source Health，不把派生索引状态伪装成源数据状态；首版不要求知识图谱、关系自动推断或 AI 摘要。
- **FR-18 本地启动与使用：** 可在本机启动 Tessera 完成发现、确认、扫描、搜索、打开来源和重建索引的完整闭环；MVP 正常使用不要求注册、登录或配置 Tessera 云服务；断网状态下文件型 Codex/Claude Code Source 的全部 MVP 功能仍可用；退出重启后 Confirmed Source、Tessera Project 和 Derived Index 仍可用。

### NonFunctional Requirements

- **NFR-1 数据所有权：** Agent Memory 始终以 Confirmed Source 为事实源，Derived Index 只能作为可重建视图。
- **NFR-2 隐私/无上传：** 正常运行不得向 Tessera 或第三方服务器上传 Agent Memory、搜索词、项目映射或诊断数据。
- **NFR-3 日志脱敏：** 默认不记录 Agent Memory 正文、搜索词或凭据到应用日志。
- **NFR-4 远程授权：** 未来远程 Knowledge Source 只能在用户显式配置和授权后由本机连接；不静默改变 MVP 隐私承诺。
- **NFR-5 最小读取范围：** Tessera 只能读取用户确认的 Source 范围，不向界面暴露任意文件读取能力。
- **NFR-6 路径边界持续校验：** Source 路径变化、符号链接或权限变化必须重新通过路径边界校验。
- **NFR-7 不可信内容安全：** 展示 Agent Memory 时按不可信内容处理，不执行其中的 HTML、脚本或命令。
- **NFR-8 故障隔离：** 单个 Source 失败不得阻断其他 Source 的搜索与浏览。
- **NFR-9 原子可见切换：** 扫描以完整成功为可见切换条件；失败时保留上一成功 Derived Index。
- **NFR-10 可恢复性：** Tessera 自有索引损坏或被删除时，可仅依赖 Confirmed Source 完整重建。
- **NFR-11 性能基准化：** 搜索延迟、首次扫描时间、内存和索引体积必须用 Carver 真实数据建立基准；基准完成前不编造固定阈值。
- **NFR-12 扫描不阻断查询：** 扫描不应阻断用户查询上一成功 Derived Index。
- **NFR-13 键盘可用：** 核心发现、搜索、筛选和来源打开操作必须支持键盘完成。

### Additional Requirements

来源：Architecture Spine（AD-1..AD-36）+ Addendum（技术机制与数据边界）。这些技术要求直接影响 Epic 与 Story 划分。

- **A-1 Phase 0 脚手架与栈锁定（→ Epic 1 Story 1）：** Bootstrap Rust（stable 1.97.x，patch 在 `rust-toolchain.toml` 锁定）内嵌同步 HTTP 服务器 crate（tiny_http 类，exact patch 由 `Cargo.lock` 持有，仅绑 127.0.0.1）+ React 19.2.7 + Vite 8.1.x + rusqlite 0.40.1（`bundled`）+ SQLite 3.x（FTS5 enabled）+ notify 8.2.x + dirs 6.x。结构种子固定：`server/src/{domain,application,adapters,index,state,policy,http}`、`src/{features,components,api}`、`tests/{ui/accessibility.spec.ts, benchmarks/memory-index.json}`。lockfile 与 toolchain 文件在 bootstrap 时拥有精确 patch。Phase 0 还须先验证并锁定 HTTP 服务器选型与 patch、端口策略、loopback 安全验收（Host/Origin 校验、CSP 响应头）、macOS 最低版本、exact toolchain build check（当前 Deferred）。（2026-07-22 起不再使用 Tauri，见 sprint-change-proposal-2026-07-22。）
- **A-2 Rust core 是唯一应用边界（AD-1）：** 所有文件访问、Provider 解析、索引写入、项目映射和查询协调必须经 Rust core application service；UI 只调用已登记的版本化 HTTP endpoint；UI 不直接依赖 Provider、文件系统或 SQLite。
- **A-3 Capability-declared ProviderAdapter 契约（AD-3, AD-25）：** 每个 Adapter 声明 `discover`/`enumerate`/`search`/`watch`/`stable_native_ids`/`coverage_level`；合约固定在 `server/src/domain/ports/provider_adapter.rs`；Adapter 输出归一化 canonical envelope（`unit_kind`、`native_unit_id`、normalized `native_locator`、title/body、scope、`source_revision`、`parser_version`）；测试夹具固定在 `server/tests/fixtures/providers/{codex,claude_code}`。
- **A-4 Confirmed Source 是唯一可读边界 + 持久 source_id + 版本化 fingerprint（AD-4, AD-33, AD-35）：** discover 只产 Candidate 元数据；确认后 core canonicalize 并保存 allowlisted root；命令只接受 `source_id`/`record_id`，不接受任意路径/SQL/句柄；每次读取重新校验仍在 root 内。Source 确认分配持久 `source_id`；re-discovery 按 `provider + canonical root fingerprint` 匹配；fingerprint 版本化（`root-fingerprint/v1`，由 provider + root kind + normalized root path + filesystem identity `(device, file_id)` 构成，identity 不可用时以 normalized path 作显式 fallback）；路径变化保留旧 Source 为 degraded 并产生新 Candidate，不自动合并；歧义/碰撞保持独立 Candidate，须显式 rebind。
- **A-5 扫描所有权与原子代际切换（AD-5, AD-16, AD-28, AD-32, AD-34, AD-36）：** 每个 Source 由单一 Scan/Reconcile owner 排队处理；扫描先写 staging generation，只有完整成功才在一次事务中切换 active generation；失败继续暴露上一成功 generation。`scan_runs` 持久化 `queued/running/staging/committing/succeeded/failed/retry` 状态机，进程启动回收 stale run。scan/reconcile 持持久单调 fencing token 与 generation intent；取消/超时/retry 后旧 owner 不得 commit；commit 在同一事务 compare-and-swap（token + intent），只有 CAS 成功才切 active。一致性级别 `snapshot-at-validation`；commit 前最终 fence/manifest 校验（size/mtime/hash + parser version）；验证后或 commit 中检测到 mutation 标记 `dirty_after_validation`，永不激活，调度有限 retry。
- **A-6 版本化 loopback-only HTTP API（AD-9, AD-17, AD-26, AD-31）：** 请求-响应用带 `api_version` 的版本化 JSON endpoint；查询统一 `cursor + limit`（server-side bound）；扫描进度用带递增 sequence 的 SSE 并支持 cancellation token；服务仅绑 127.0.0.1、校验 Host/Origin、携带 CSP 响应头，不监听外部接口，不开放 WebSocket/远程 URL。cursor 携带 generation、projection revisions、sort key、record_id；snapshot token 绑定 active generation + project_mapping_revision + filter/policy revision + sort key；任一 revision 变化返回 `stale_snapshot`。
- **A-7 Derived Index = SQLite/FTS5，migration 版本化（AD-2, AD-29）：** Tessera SQLite、Source Registry 状态、项目映射属 Tessera 自有数据，可删除可重建，禁止回写 Source。Reset Index 清理 canonical body、FTS、scan runs 但保留 Source Registry 与 Tessera Project mappings；移除 Source 清理其派生 records；migration 原子执行并失败保留旧 index。
- **A-8 结构化错误信封（AD-13）：** core 拥有共享 error envelope（stable `code` + safe `message` + `source_id` + phase）；Source 失败从不影响无关 Source generation；错误展示不含正文/凭据。
- **A-9 浏览与搜索共享查询契约（AD-23）：** Query Service 提供版本化 `BrowsePage`/`SearchPage`，统一 `cursor`、`limit`、stable sort、`EmptyState` enum、Coverage Level、Source Health metadata；Browse 不绕过 Query Service 直读索引表。
- **A-10 Canonical 身份基于 locator（AD-6, AD-15, AD-30）：** `record_id` 由 `source_id + provider + native locator + unit kind` 稳定生成；content hash 只用于变化检测；parser version 只作解析版本与重建触发；file line range 只用于打开/展示，不参与身份；`native_unit_id`（provider id / heading path + duplicate ordinal / file-level fallback）稳定性由 fixture 固定；无法稳定拆分按 file-level unit，不宣称 section identity。
- **A-11 Tessera Project 映射基数与优先级（AD-24, AD-27）：** Native Project/Provider scope 默认隔离，未知 scope 不自动合并；一个 Native Project 在一个 mapping scope 至多属于一个 active Tessera Project；只有显式 mapping 生效；projection 不复制 canonical records、不改 native identity。
- **A-12 Watcher 是 hint，reconcile 是真相（AD-8）：** watcher 只产生按 Source debounce 的 dirty hint；reconcile 通过受限扫描、size/mtime/hash、parser version 判断变化；定期 reconcile 修复漏事件；事件本身不直接增删 canonical records。
- **A-13 Adapter 契约与安全测试门禁（AD-14）：** Codex 与 Claude adapter 各需 fixture contract tests、zero-source-mutation tests、parser-version tests、reconcile recovery tests、capability honesty tests 通过后才能在默认构建启用。
- **A-14 Local-only 强制 + 单机运行边界（AD-12, AD-20）：** MVP 无出站网络路径；日志 omit body/query/credential；Phase A 仅支持 Carver 当前本机单一本地服务进程（Rust 二进制 + 用户默认浏览器）；Source roots 只读且位于应用外部，Tessera index/config/scan state 位于 OS-managed app-data（经 dirs crate 解析）；公开签名、自动更新、跨平台分发、远程服务 Deferred。
- **A-15 不可信内容渲染 + 入站拒绝（AD-11 + Deferred）：** Adapter 在 canonicalization 前拒绝 raw chat/session/transcript 与人工指令文件；Markdown 与所有 Agent Memory 按不可信内容安全渲染。CSP/Markdown sanitizer、FTS5 中文 tokenizer 与搜索基线、外部 SQLite `mode=ro`/WAL sidecar 条件由 Phase 0 与安全测试 owner 先验证再决定是否提升为新 AD，当前不得用未验证便利实现绕过。
- **A-16 性能基准是质量门禁（AD-22）：** Phase 0 固定匿名 fixture，记录 cold scan、query、memory、index-size baseline；结果文件固定 `tests/benchmarks/memory-index.json`，由 Phase 0 owner 生成并锁定阈值；后续变更须报告同一 fixture 回归并通过 gate 才进默认构建。
- **A-17 UI 可访问性是共享交互契约（AD-21）：** Inventory、Browse、Search、Health、Provenance 共享语义 focus order、keyboard-reachable commands、可读状态标签、EmptyState；视觉组件不得成为唯一可用入口；验收产物固定 `tests/ui/accessibility.spec.ts`。
- **A-18 Supported Artifact Matrix（数据边界，AD-11 + Addendum 2.1）：
  - **Codex 纳入：** 默认 `~/.codex/memories` 或 `CODEX_HOME/memories` 下自动生成 Markdown：`MEMORY.md`、`memory_summary.md`、`raw_memories.md`、`rollout_summaries/*.md`；**排除：** rollout/transcript JSONL、session 内容、状态库对话内容、root 外目录。
  - **Claude Code 纳入：** 官方默认 `~/.claude/projects/<project>/memory/` 与用户 `autoMemoryDirectory` 的 `MEMORY.md` 和 topic Markdown；**排除：** `CLAUDE.md`、`AGENTS.md`、`.claude/rules`、session/transcript、任意手动目录。
  - 未知文件记录 `unsupported_artifact` 诊断但不索引；格式变化进入 fixture 与 parser-version 流程。
- **A-19 Agent Memory 与 Knowledge Source 不同领域（AD-10, AD-19）：** Source 统一注册须带 `source_kind: agent_memory | local_knowledge | remote_knowledge`；MVP 只实现 `agent_memory`；未来 Knowledge Source 独立 namespace/identity prefix/parser registry/migration/query filter，只复用 registry/index/query ports 基础能力，不共享 Agent Memory canonical table 或写入语义。
- **A-20 CAP 交叉校验（SPEC）：** SPEC 的 CAP-1..CAP-11 与 FR 一一对应（CAP-1→FR-1；CAP-2→FR-2；CAP-3→FR-3；CAP-4→FR-4/5；CAP-5→FR-6；CAP-6→FR-7/8/15；CAP-7→FR-9/10；CAP-8→FR-11/12；CAP-9→FR-13/14；CAP-10→FR-16/17；CAP-11→FR-18），作为完整性交叉校验，无新增需求。

### UX Design Requirements

无独立 UX 设计契约（`DESIGN.md`/`EXPERIENCE.md` 缺失）。经用户确认采用「继续，UX 决策嵌入 Story」策略：下列推迟到 UX/实现阶段的决策，将在对应 Story 中以"待定 UX 决策"或 dev 阶段决策形式嵌入，并在 AC 中标注可验证的功能性约束（视觉/交互细则待 UX 阶段补强）。

- **UX-DR1 Source 发现与首次确认流程：** Candidate Source 展示发现依据与路径、逐个确认/拒绝交互、无候选空态（FR-1/FR-2；UJ-1）。
- **UX-DR2 Source Inventory 状态展示：** Provider、Coverage Level、Source Health、最近扫描、记录数、最近错误的结构化状态卡片与 EmptyState（FR-3/FR-13；AD-7）。
- **UX-DR3 Tessera Project 创建与 Native Project 关联 UI 流程：** PRD 开放问题 #2 推迟项 —— 显式创建 + 多对一关联 + 调整/移除映射的交互（FR-5；AD-24/AD-27）。
- **UX-DR4 Agent Memory scope 建模与默认隔离展示：** PRD 开放问题 #6 推迟项 —— Provider 原生 scope 保留与未知 scope 不合并的呈现（FR-4；AD-24）。
- **UX-DR5 搜索/筛选交互：** 关键词输入、组合筛选范围可见性、清除筛选、空结果三态（无匹配/未索引/不可用）区分（FR-9/FR-10；AD-7）。
- **UX-DR6 结果卡片与 Provenance 视图：** 原始片段展示、完整 Provenance 字段、Coverage Level/Source Health 标注、打开原始位置（FR-11/FR-12；AD-6）。
- **UX-DR7 浏览与记忆结构可视化：** 分页列表、按时间/Native Project/Memory 类型分组、从 Provider→Project→Memory→原始位置的导航（FR-16/FR-17）。
- **UX-DR8 键盘可达性与共享交互契约：** 发现/搜索/筛选/打开核心路径键盘完成、共享 focus order、可读状态标签、EmptyState（NFR-13；AD-21；验收产物 `tests/ui/accessibility.spec.ts`）。

> 后续若产出 UX 设计契约（`/bmad:bmm:workflows:ux`），应将其中可实施项补录为 UX-DR9..n 并据此细化相关 Story 的 AC。

### FR Coverage Map

- **FR-1** → Epic 1 — 自动发现 Codex Candidate Source（Provider、候选路径、发现依据、Native Project；不读聊天记录）
- **FR-2** → Epic 1 — 确认/拒绝/停用 Codex Source（确认状态重启保留；停用不改原始记忆）
- **FR-3** → Epic 1 + Epic 2 — Source Inventory（E1 Codex 行；E2 多 Provider 全景含数量/覆盖/健康/最近扫描）
- **FR-4** → Epic 1 — 保留 Native Project（native identity 原样保留，无法确认显示未映射）
- **FR-5** → Epic 5 — 建立 Tessera Project 映射（跨 Provider 多对一关联，仅写本地状态）
- **FR-6** → Epic 1 — 限定 Codex Agent Memory 边界（排除聊天/transcript/规则文件；记录 Memory 类型）
- **FR-7** → Epic 1 — 以只读方式建立 Codex Derived Index（源不变；失败不替换上一成功版本）
- **FR-8** → Epic 1（手动重扫）+ Epic 4（变化检测 + watcher/reconcile 自动刷新）
- **FR-9** → Epic 1 + Epic 2 — 搜索 Confirmed Source（E1 Codex；E2 跨 Codex+Claude Code）
- **FR-10** → Epic 2 — 筛选搜索结果（跨 Provider/Source/Project/Native Project/类型/时间组合，范围可见）
- **FR-11** → Epic 1 — 展示原始结果与 Provenance（Provider/Source/Native Project/原始位置/定位/时间/Coverage/Health）
- **FR-12** → Epic 1 — 打开原始位置（只打开不编辑；失效明示错误 + Health）
- **FR-13** → Epic 1 + Epic 2 + Epic 4 — Source Health（E1 Codex；E2 多源；E4 失败态完整：路径/权限/格式/扫描失败）
- **FR-14** → Epic 4 — 隔离 Connector 失败（单源失败不阻断其他；上一成功结果标 stale）
- **FR-15** → Epic 4 — 重建 Derived Index（删除重建不丢 Source 与 Project 映射）
- **FR-16** → Epic 3 — 浏览 Agent Memory 集合（分页、筛选、空集合三态区分）
- **FR-17** → Epic 3 — 可视化记忆结构（Provider→Project→Memory→原始位置 drill-down）
- **FR-18** → Epic 1（基础，全部 Epic 共享离线约束）— 本地启动与使用（无账号、断网可用、重启保留）

## Epic List

### Epic 1: 本机 Codex 记忆发现与搜索（Foundation + 首个端到端闭环）

Carver 在本机启动 Tessera，自动发现 Codex 的 Agent Memory 来源，逐个确认，建立只读 Derived Index，按关键词搜索并从结果卡片追溯到原始记忆位置 —— 端到端跑通单 Provider 的"看见 + 找到 + 打开"。
**FRs covered:** FR-1, FR-2, FR-3, FR-4, FR-6, FR-7, FR-9, FR-11, FR-12, FR-18（FR-8 手动重扫、FR-13 Codex 健康在此引入）
**关键载入:** A-1 Phase 0 脚手架与栈锁定、A-2 Rust core 唯一边界、A-3 ProviderAdapter 契约、A-4 路径策略 + 持久 source_id + 版本化 fingerprint、A-5 原子代际切换、A-6 版本化 HTTP API、A-7 SQLite/FTS5 Derived Index、A-8 错误信封、A-10 locator 身份、A-13 Codex 五类契约测试、A-14 local-only、A-15 不可信渲染、A-16 基准门禁骨架、A-18 Codex artifact matrix。
**风险边界:** 验证整个引擎 + Codex 真实记忆格式解析；早期反馈固化 adapter 契约后再接入 Claude Code。
**依赖:** 无（基础 Epic；Story 1 = Phase 0 脚手架）。

### Epic 2: 跨 Agent 联邦（Claude Code + 跨 Provider 搜索与全景 Inventory）

Carver 把 Claude Code 的 Agent Memory 也接入，在 Source Inventory 同时看到两个 Provider 的范围、覆盖与健康；在一个查询里跨 Codex + Claude Code 搜索，按 Provider/Source/Project/Native Project/类型/时间筛选，并对比每条结果的来源 —— 实现产品核心的跨 Agent 差异化。
**FRs covered:** FR-10（+ Claude Code 经 Epic 1 固化契约接入；FR-3/FR-13 多 Provider 全景在此完成）
**关键载入:** Claude Code adapter + 五类契约测试、Source Inventory 多 Provider 完整视图、Coverage Level/Source Health 跨源展示、跨 Provider 组合筛选与范围可见性、空结果三态跨源区分。
**依赖:** Epic 1（复用已固化 adapter 契约、索引、Query Service、IPC）。

### Epic 3: 无查询浏览与记忆结构可视化

Carver 在没有查询词时，按 Provider/Tessera Project/Native Project/时间/Agent Memory 类型浏览记忆集合与层级结构，从 Provider 钻取到项目、记忆条目和原始位置，理解各来源记忆的范围与最近变化。
**FRs covered:** FR-16, FR-17
**关键载入:** Query Service BrowsePage（与 SearchPage 共享 cursor/limit/sort/EmptyState/Health 契约 A-9/A-23）、分组与状态视图、drill-down 导航、UI 可访问性共享契约（A-17，验收 `tests/ui/accessibility.spec.ts`）。
**依赖:** Epic 1/2（Query Service 与已索引内容）。

### Epic 4: 健康诊断、失败隔离与索引重建

当某个 Connector 失效（路径移动、权限变化、格式不支持、扫描失败），其他 Source 仍可查询；失败 Source 明确展示原因、上次成功时间与 stale 状态；用户可触发重新扫描，或整体删除重建 Derived Index 而不丢失 Confirmed Source 与 Tessera Project 映射。
**FRs covered:** FR-8（变化检测 + 自动刷新）, FR-13（失败态完整）, FR-14, FR-15
**关键载入:** A-5/A-12 watcher-as-hint + reconcile 为真相、A-8 Source-scoped 错误隔离、stale generation 保留与可见、AD-33 路径变化 degraded→新 Candidate、A-29 Reset Index 保留 Registry/映射的边界。
**依赖:** Epic 1/2（代际切换机制、Source Registry）。

### Epic 5: Tessera Project 跨 Agent 项目联邦视图

Carver 创建 Tessera Project，把来自 Codex 与 Claude Code 的多个 Native Project 显式关联到同一项目视图，按 Tessera Project 浏览与搜索，而 native identity 与源数据从不被改动。
**FRs covered:** FR-5（FR-4 native identity 自 Epic 1 起保留，本 Epic 增加显式映射投影）
**关键载入:** AD-24/AD-27 显式映射基数与优先级、projection 不复制 canonical records、project_mapping_revision 纳入 cursor snapshot（A-6/A-9/A-31）。
**依赖:** Epic 1/2（多 Provider native identity）。

> Epic 顺序说明：架构 Spine 为 `final`（高确定性）→ 采用少而大的 Epic；Codex-first（E1→E2）作为风险降级切片，先用真实格式解析固化 adapter 契约再接入 Claude Code；韧性（E4）排在项目联邦（E5）之前以先建立信任基础。E4/E5 顺序可按 Carver 单人本机实际优先级调整。

## Epic 1: 本机 Codex 记忆发现与搜索（Foundation + 首个端到端闭环）

Carver 在本机启动 Tessera，自动发现 Codex 的 Agent Memory 来源，逐个确认，建立只读 Derived Index，按关键词搜索并从结果卡片追溯到原始记忆位置 —— 端到端跑通单 Provider 的"看见 + 找到 + 打开"。

### Story 1.1: 本地应用骨架与可启动运行（Phase 0 脚手架）

As a Carver,
I want 在本机启动一个可运行的 Tessera 桌面应用骨架,
So that 后续所有功能都能在一个离线、无账号、Rust-core 为唯一边界的 shell 上构建。

**Acceptance Criteria:**

**Given** 一台 macOS 本机
**When** Carver 构建并启动应用
**Then** Rust core（domain/application/adapters/index/state/policy/http 模块骨架）内嵌同步 HTTP 服务（仅绑 127.0.0.1）+ React 19/Vite 8 浏览器 UI + rusqlite(bundled, FTS5) + notify 就绪，并在启动后自动打开默认浏览器访问本地地址
**And** 一个带 `api_version` 的 ping endpoint 能 UI→core→UI 往返
**And** 启动过程无任何出站网络请求（NFR-2），服务不监听任何非回环地址（lsof 核验），响应携带收紧 CSP 头并校验 Host/Origin（AD-9/AD-12），`rust-toolchain.toml` 锁定 stable patch
**And** `cargo test` 与 `npm run build` 通过，FTS5 可用，migration 框架就绪（v0）
**And** 预留 `tests/benchmarks/memory-index.json` 与 `tests/ui/accessibility.spec.ts` 占位（A-16/A-17）
**And** Phase 0 验证 A-15 Deferred 项并记录结论：FTS5 中文 tokenizer 在真实样本上的分词/短查询行为、Markdown 与 Agent Memory 不可信内容的 CSP/sanitizer 方案（CSP 以 HTTP 响应头落地）、外部 SQLite `mode=ro`/WAL sidecar 可行性 —— 结论作为 1.5/1.6 实现路径与是否提升为新 AD 的依据（回应 PRD 开放问题 #3，闭环 readiness m1）

### Story 1.2: Codex Candidate Source 自动发现与展示

As a Carver,
I want 启动后自动发现本机 Codex Agent Memory 来源并列出,
So that 我知道有哪些可接入的记忆资产。

**Acceptance Criteria:**

**Given** 本机存在 `~/.codex/memories` 或 `CODEX_HOME/memories`
**When** 应用启动发现
**Then** UI 列出 Candidate Source，显示 Provider、候选路径、发现依据和可判定 Native Project
**And** 发现阶段不读取任何聊天/transcript 内容（NFR-5），Codex adapter 声明 `discover` + `coverage_level`（A-3）

**Given** 本机无受支持 Codex 来源
**When** 启动发现
**Then** 显示空态且**不**提供手动添加目录入口

### Story 1.3: Source 确认/拒绝/停用与持久身份

As a Carver,
I want 逐个确认或拒绝 Candidate Source 并能停用,
So that 只有我允许的来源才被读取，且决定在重启后保留。

**Acceptance Criteria:**

**Given** 一个 Candidate Source
**When** Carver 确认
**Then** core canonicalize root、保存 allowlist、分配持久 `source_id` 与版本化 fingerprint（`root-fingerprint/v1`），Source 变为 Confirmed
**And** 确认状态在应用重启后保留；拒绝的来源不进索引；停用已确认 Source 不修改原始记忆（NFR-1）
**And** 后续命令只接受 `source_id`，不接受任意路径（NFR-5/6）；re-discovery 按 `provider + fingerprint` 匹配，路径变化保留旧 Source 为 degraded（AD-33/AD-35）

### Story 1.4: 只读扫描管线与原子代际切换（骨架）

As a Carver,
I want 扫描以"完整成功才可见、失败保留上一成功版本"的方式建立索引,
So that 我永远看不到半套或失败覆盖的结果。

**Acceptance Criteria:**

**Given** 一个 Confirmed Codex Source（先用 file-level unit 解析作为基线，AD-30）
**When** 触发扫描
**Then** 先写 staging generation，只有完整成功才在一次事务中 CAS 切换 active generation（fencing token）

**Given** 扫描中途失败
**When** 失败发生
**Then** 上一成功 active generation 继续可见，不出现半套索引（NFR-9）
**And** `scan_runs` 持久化 `queued/running/staging/committing/succeeded/failed/retry`；进程启动回收 stale run（AD-16）；`dirty_after_validation` generation 永不激活（AD-36）
**And** 扫描前后源文件集合/内容/大小/修改时间不变（零写入测试，SM-2）

### Story 1.5: Codex 记忆解析、边界限定与 canonical 记录

As a Carver,
I want Tessera 只把 Codex 自动生成的记忆工件解析成带稳定身份与 Provenance 的 canonical 记录,
So that 索引里只有真正的 Agent Memory，且每条可追溯。

**Acceptance Criteria:**

**Given** Codex 记忆目录含 `MEMORY.md`/`memory_summary.md`/`raw_memories.md`/`rollout_summaries/*.md`
**When** 解析
**Then** 产出 canonical 记录（`unit_kind`、`native_unit_id` = heading path + duplicate ordinal / file-level fallback、normalized `native_locator` = file URI + 展示用 line range、title/body、native project、`source_revision`、`parser_version`、Provider Memory 类型）

**Given** 文件是 rollout/transcript JSONL、session 内容、状态库对话或 `CLAUDE.md`/`AGENTS.md`/规则文件
**When** canonicalization
**Then** 被拒绝，不进索引（A-11/A-18）
**And** 未知文件记录 `unsupported_artifact` 诊断但不索引；Native Project 原样保留，无法确认时显示"未映射"而非猜测（FR-4）
**And** `record_id` 由 `source_id + provider + native locator + unit kind` 稳定生成；content hash 仅用于变化检测
**And** Codex 五类契约测试通过（A-13）：fixture contract / zero-source-mutation / parser-version / reconcile-recovery / capability-honesty

### Story 1.6: 关键词搜索与 Provenance 结果展示

As a Carver,
I want 输入关键词搜索 Codex 记忆并看到原始片段 + 完整来源,
So that 我能核验信息而非依赖合成答案。

**Acceptance Criteria:**

**Given** Codex Source 已成功索引
**When** Carver 输入关键词
**Then** Query Service `SearchPage`（`cursor + limit`、`api_version`、stable sort）返回带原始片段的结果卡片，每条含完整 Provenance（Provider/Source/Native Project/原始 locator/更新时间/Coverage Level/Source Health）（FR-11）
**And** 查询不调用外部模型或远程搜索（NFR-2）；空结果区分"确实无匹配 / Source 未索引 / Source 当前不可用"三态
**And** Coverage Level/Source Health 字段不把派生状态伪装成源事实；cursor 分页正常

### Story 1.7: 从结果打开原始记忆位置

As a Carver,
I want 从结果卡片打开或定位原始 Agent Memory 文件,
So that 我能直接看到原始上下文。

**Acceptance Criteria:**

**Given** 一条搜索结果
**When** Carver 点击"打开原始位置"
**Then** core 按 `record_id` 解析 origin locator、重新校验仍在 allowlisted root 内（NFR-6）、由服务端调用 OS（macOS `open`）在对应行打开/定位；浏览器不直接接触文件系统能力
**And** Tessera 只打开/定位，不在应用内编辑原始文件（NFR-1）

**Given** 原始位置已失效（文件移动/删除/权限）
**When** 尝试打开
**Then** 展示可理解错误 + 当前 Source Health 状态，且错误不含正文/凭据（NFR-3）

### Story 1.8: Source Inventory、健康状态与手动重扫

As a Carver,
I want 在 Inventory 看到 Codex Source 的范围/覆盖/健康/数量/最近错误，并能手动重扫,
So that 我判断结果是否可信并按需刷新。

**Acceptance Criteria:**

**Given** 一个 Confirmed Codex Source
**When** Carver 打开 Inventory
**Then** 显示 Provider、路径、Native Project、Coverage Level、Source Health（`unknown/healthy/degraded/error`）、最近成功扫描时间、记录数量、最近错误
**And** 数量仅在能完整枚举时显示为完整；`search_only/existence_only/unsupported` 不显示为"完整同步"；Health 变化不删除确认关系
**And** Health 原因至少区分路径失效/权限不足/格式不支持/扫描失败，且错误展示不含正文/凭据（NFR-3）

**Given** Carver 点"重新扫描"
**When** 触发
**Then** 只重扫该 Confirmed Source，扫描进度通过带递增 sequence 的 SSE 可见、可取消（A-6）
**And** Inventory/搜索/打开核心操作支持键盘完成（NFR-13，UX-DR8）

### Story 1.9: Phase 0 性能基准门禁

As a Carver,
I want 用我的真实数据建立 cold scan / query / memory / index-size 性能基准并锁定为回归门禁,
So that 后续变更不会悄悄让扫描或搜索变慢。

**Acceptance Criteria:**

**Given** Epic 1 的扫描（Story 1.4/1.5）与关键词搜索（Story 1.6）已可用
**When** Phase 0 owner 在固定匿名 fixture（Carver 真实 Codex 记忆样本）上运行基准
**Then** 生成 `tests/benchmarks/memory-index.json`，记录 cold scan 时间、query 延迟、内存占用、index 体积四项 baseline
**And** 基准完成前不编造固定阈值（NFR-11）；阈值由 Phase 0 owner 基于真实数据锁定后写入该文件
**And** 后续任何变更（Epic 2+）须报告同一 fixture 的回归，未通过 gate 不进默认构建（A-16/AD-22）

## Epic 2: 跨 Agent 联邦（Claude Code + 跨 Provider 搜索与全景 Inventory）

Carver 把 Claude Code 的 Agent Memory 也接入，在 Source Inventory 同时看到两个 Provider 的范围、覆盖与健康；在一个查询里跨 Codex + Claude Code 搜索，按 Provider/Source/Project/Native Project/类型/时间筛选，并对比每条结果的来源 —— 实现产品核心的跨 Agent 差异化。

### Story 2.1: Claude Code Candidate Source 发现与确认

As a Carver,
I want 发现并确认本机 Claude Code 的 auto-memory 来源,
So that 把第二个 Agent 的记忆也纳入 Tessera。

**Acceptance Criteria:**

**Given** 本机存在 `~/.claude/projects/<project>/memory/` 或用户 `autoMemoryDirectory`
**When** 启动发现
**Then** 列出 Claude Code Candidate Source（Provider、路径、发现依据、Native Project），声明 `coverage_level`
**And** 确认/拒绝/停用复用 Epic 1 Source Registry + fingerprint；Claude 与 Codex Source 在同一 Registry 共存；确认持久、重启保留

### Story 2.2: Claude Code 记忆解析、边界限定与只读索引

As a Carver,
I want Tessera 只把 Claude Code 自动生成的 `MEMORY.md` 与 topic Markdown 解析进索引,
So that Claude 记忆可被搜索且原始文件不被改动。

**Acceptance Criteria:**

**Given** Claude auto-memory 目录含 `MEMORY.md` 与 topic Markdown
**When** 扫描
**Then** 经 Epic 1 原子代际管线产出 canonical 记录（provider=`claude_code`、`parser_version`、topic/heading 身份）

**Given** 文件是 `CLAUDE.md`/`AGENTS.md`/`.claude/rules`/session/transcript 或手动添加目录
**When** canonicalization
**Then** 被拒绝，不进索引（A-18）
**And** 扫描只读，源文件不变（零写入测试）；Claude 五类契约测试通过（A-13）；`autoMemoryDirectory` 解析正确

### Story 2.3: 跨 Provider 关键词搜索与来源对比

As a Carver,
I want 一个查询同时搜 Codex 和 Claude Code 并看清每条结果来自哪个 Agent,
So that 我跨 Agent 恢复上下文并核验出处。

**Acceptance Criteria:**

**Given** Codex 与 Claude Code 均已成功索引
**When** Carver 输入关键词
**Then** 默认搜索所有健康且已索引的 Confirmed Source，结果按相关性排序，每张卡片标注 Provider + 完整 Provenance，可对比同一查询两个 Agent 各自的记忆
**And** 查询不调用外部模型（NFR-2）；单个 Source 不可用时其结果标记不可用，其他 Source 结果照常返回（FR-14 雏形）

### Story 2.4: 跨 Provider 组合筛选与范围可见性

As a Carver,
I want 按 Provider/Source/Native Project/Memory 类型/时间组合筛选并看到当前生效范围,
So that 我精确缩小到关心的记忆切片。

**Acceptance Criteria:**

**Given** 跨源搜索结果
**When** Carver 组合筛选（Provider + Memory 类型 + 时间等）
**Then** 结果即时收敛，界面显示当前生效范围（如"Codex + Claude Code，类型=MEMORY，近 7 天"）
**And** 清除筛选恢复全部 Confirmed Source 范围；Native Project 筛选可跨 Provider 生效；Tessera Project 筛选位预留（Epic 5 填充）

### Story 2.5: 多 Provider Source Inventory 全景与跨源健康

As a Carver,
I want 在一个 Inventory 同时看到 Codex 和 Claude Code 所有 Source 的范围/覆盖/健康/数量,
So that 我判断整体可信度。

**Acceptance Criteria:**

**Given** 多个 Confirmed Source（Codex + Claude Code）
**When** Carver 打开 Inventory
**Then** 列出所有 Source 的 Provider、路径、Native Project、Coverage Level、Source Health、最近成功扫描、记录数量、最近错误
**And** 数量按各自 Coverage 诚实展示；一个 Source 失败不影响其他 Source 的展示与状态；跨源健康状态可对比

## Epic 3: 无查询浏览与记忆结构可视化

Carver 在没有查询词时，按 Provider/Tessera Project/Native Project/时间/Agent Memory 类型浏览记忆集合与层级结构，从 Provider 钻取到项目、记忆条目和原始位置，理解各来源记忆的范围与最近变化。

### Story 3.1: BrowsePage 查询契约与无查询浏览入口

As a Carver,
I want 不输入查询词直接进入记忆集合浏览分页列表,
So that 我先了解某 Agent 记忆的范围而不必猜关键词。

**Acceptance Criteria:**

**Given** 一个已成功扫描的 Source
**When** Carver 从 Source Inventory 进入浏览
**Then** Query Service `BrowsePage`（与 SearchPage 共享 cursor/limit/sort/EmptyState/Health，A-23）返回分页记忆列表，不绕过 Query Service 直读索引
**And** 空集合区分"尚未扫描 / 没有可索引 Agent Memory / Source 当前不可用"三态；列表不含原始聊天、人工指令文件或未确认 Source

### Story 3.2: 按维度分组与最近变化浏览

As a Carver,
I want 按时间、Native Project、Memory 类型、Provider 分组浏览并看最近变化,
So that 我快速定位关心的记忆切片。

**Acceptance Criteria:**

**Given** 浏览集合
**When** Carver 按维度分组/筛选
**Then** 可按 Provider/Native Project/Memory 类型/时间分组，并查看最近变化视图
**And** 浏览结果与搜索结果共用同一 Provenance、Coverage Level、Source Health 字段（A-23）

### Story 3.3: 记忆结构 drill-down 导航与可视化

As a Carver,
I want 从 Provider 钻取到 Native Project、记忆条目和原始位置,
So that 我理解各来源记忆的层级与状态。

**Acceptance Criteria:**

**Given** 多 Provider 已索引内容
**When** Carver 导航
**Then** 可 Provider → Native Project → 记忆条目 → 打开原始位置（复用 Epic 1 open）
**And** 视图显示最近扫描、最近变化、Source Health，**不**把派生索引状态伪装成源数据状态；首版不要求知识图谱、关系自动推断或 AI 摘要（FR-17 边界）

## Epic 4: 健康诊断、失败隔离与索引重建

当某个 Connector 失效（路径移动、权限变化、格式不支持、扫描失败），其他 Source 仍可查询；失败 Source 明确展示原因、上次成功时间与 stale 状态；用户可触发重新扫描，或整体删除重建 Derived Index 而不丢失 Confirmed Source 与 Tessera Project 映射。

### Story 4.1: 文件变化 watcher hint 与 reconcile 自动刷新

As a Carver,
I want Source 内容变化后索引自动更新,
So that 搜索结果反映最新记忆。

**Acceptance Criteria:**

**Given** 一个 Confirmed Source 的记忆文件变化
**When** notify watcher 产生按 Source debounce 的 dirty hint
**Then** reconcile 通过受限扫描 + size/mtime/hash + parser_version 判断变化，成功后 add/modify/delete 反映到查询
**And** watcher 事件本身不直接增删 canonical records（A-12）；定期 reconcile 修复漏事件；扫描期间上一成功 Derived Index 仍可查询（NFR-12）

### Story 4.2: Connector 失败隔离与 stale 上一成功结果

As a Carver,
I want 某个 Connector 失效时其他 Source 仍可查且失败原因明确,
So that 我不把静默失败当成没有记忆。

**Acceptance Criteria:**

**Given** Codex 与 Claude Code 均有可用索引
**When** 人为使其中一个 Source 不可读（路径移动/权限/格式/扫描失败）
**Then** 另一个 Source 仍可搜索和浏览（NFR-8）
**And** 失败 Source 标记 degraded/error + 可理解原因 + 上次成功时间 + stale；上一成功 generation 不被半次失败扫描覆盖（NFR-9）；错误展示不含正文/凭据（NFR-3）

### Story 4.3: 路径/权限/身份变化的重发现与 degraded 处理

As a Carver,
I want Source 路径或权限变化时看到明确状态而非重复或丢失,
So that 我的确认关系和映射稳定。

**Acceptance Criteria:**

**Given** 一个 Confirmed Source 的 root 被移动/权限或身份变化
**When** 重新发现
**Then** 旧 Source 保留为 degraded 并展示原因与上次成功时间，同时产生新 Candidate
**And** 不自动合并或复制 Tessera Project mapping；只有显式 rebind 才改变 root（AD-33/AD-35）；歧义 fingerprint 保持独立 Candidate

### Story 4.4: Derived Index 整体重建

As a Carver,
I want 删除并完整重建 Derived Index 而不丢 Source 确认与 Project 映射,
So that 索引损坏时可恢复。

**Acceptance Criteria:**

**Given** Carver 触发重建
**When** 执行
**Then** 重建前明确告知只会删除 Tessera 派生数据；Reset 清理 canonical body/FTS/scan_runs 但保留 Source Registry 与 Tessera Project mappings（A-29）
**And** 重建后恢复相同来源记录的稳定身份与 Provenance；重建失败时原始 Agent Memory 保持不变（NFR-1/10）

## Epic 5: Tessera Project 跨 Agent 项目联邦视图

Carver 创建 Tessera Project，把来自 Codex 与 Claude Code 的多个 Native Project 显式关联到同一项目视图，按 Tessera Project 浏览与搜索，而 native identity 与源数据从不被改动。

### Story 5.1: Tessera Project 创建与 Native Project 显式映射

As a Carver,
I want 创建 Tessera Project 并把 Codex 与 Claude Code 的多个 Native Project 关联到同一项目,
So that 我用一个视图看同一真实项目的跨 Agent 记忆。

**Acceptance Criteria:**

**Given** 多个 Provider 的 Native Project
**When** Carver 创建 Tessera Project 并关联
**Then** 映射仅存于本地状态，不修改 Provider 目录/文件
**And** 可查看/调整/移除映射且不删除任何 Agent Memory 或索引记录；一个 Native Project 在一个 mapping scope 至多属于一个 active Tessera Project（AD-27）；未知映射保持未映射，不自动合并（AD-24）

### Story 5.2: 按 Tessera Project 浏览与搜索（projection）

As a Carver,
I want 按 Tessera Project 浏览和搜索其聚合的记忆,
So that 我聚焦一个真实项目的全部跨 Agent 记忆。

**Acceptance Criteria:**

**Given** 一个含多个 Native Project 映射的 Tessera Project
**When** Carver 选定它浏览/搜索
**Then** 结果只含其映射 Native Project 的记忆（projection 不复制 canonical records、不改 native identity）
**And** 映射变化时 cursor snapshot 返回 `stale_snapshot`，调用方从新快照重新分页（A-6/A-31）；Epic 2 预留的 Tessera Project 筛选位在此填充

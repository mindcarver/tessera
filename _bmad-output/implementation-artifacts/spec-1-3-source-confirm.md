---
title: 'Source 确认/拒绝/停用与持久身份'
type: 'feature'
created: '2026-07-22'
status: 'done'
baseline_revision: 'NO_VCS'
final_revision: 'NO_VCS'
review_loop_iteration: 0
followup_review_recommended: false
context:
  - '_bmad-output/implementation-artifacts/epic-1-context.md'
  - '_bmad-output/planning-artifacts/architecture/architecture-tessera-2026-07-20/ARCHITECTURE-SPINE.md'
  - '_bmad-output/implementation-artifacts/spec-1-2-codex-discover.md'
  - '_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md'
warnings:
  - oversized
---

<intent-contract>

## Intent

**Problem:** Story 1.2 让 Tessera 能"看见"本机 Codex Candidate Source，但候选是每次启动重新发现的临时态——没有 `source_id`、不持久化、不 canonicalize、无 fingerprint。Carver 无法选择"只读哪些来源"，决定也不跨重启保留；后续扫描/索引（1.4/1.5）缺少"已确认 Source"这一唯一可读边界。

**Approach:** 引入 Source Registry（Tessera 自有 SQLite 持久化，新 migration v1）与 `domain::source` 身份类型；`policy` 提供 root canonicalize（归一化路径 + 文件系统 identity `(device, file_id)`）；`application::source` 编排 confirm/reject/disable/list，确认时分配持久 `source_id`、计算版本化 fingerprint（`root-fingerprint/v1`）、按 fingerprint 幂等匹配。新增版本化 Tauri command：`confirm_source`/`reject_source`（接受 CandidateSource，是唯一接受路径的"入边界"命令）、`disable_source`（只接受 `source_id`）、`list_sources`。UI 把候选列表升级为可逐个确认/拒绝、并展示已注册 Source（可停用），键盘可达。

## Boundaries & Constraints

**Always:**
- AD-1：Rust core 唯一边界。confirm/reject/disable/list 经 `application::source` → `index::SourceRegistry` → SQLite；UI 只调已登记的 typed Tauri command，不直接访问 FS/SQLite/Provider。
- AD-4：`confirm`/`reject` 是唯一接受路径（CandidateSource）的命令——即"allowlist 入边界"动作；`disable`/`list` 等后续命令只接受 `source_id` 或无参，不接受任意路径。确认时由 core canonicalize root 并保存 allowlisted root。
- AD-33/AD-35：确认分配持久 `source_id`；fingerprint 版本化 `root-fingerprint/v1`，由 `provider + root kind + normalized root path + filesystem identity (device, file_id)` 构成，identity 不可用时以 normalized path 作显式 fallback；匹配为**精确相等**，不做 fuzzy merge；路径/inode 变化产生**不同** fingerprint → 不同 Source（旧 Source 保留，不自动合并）。
- AD-7：lifecycle / health / coverage 分开建模。`lifecycle_state ∈ {confirmed, disabled, rejected}`；`health_state` 列存在但 1.3 恒写 `unknown`（health 追踪属 1.8/4.x）。
- A-19：Registry 行带 `source_kind='agent_memory'`（MVP 唯一取值）。
- AD-13/NFR-3：失败用结构化 ErrorEnvelope（stable `code` + safe `message`），不含正文/查询词/凭据。1.3 新增 stable code `confirm_failed`、`source_not_found`（复用既有 `internal`/`ipc_contract`）。
- NFR-1：confirm/reject/disable 只写 Tessera 自有 SQLite，永不写 Source 文件（零写入）。
- NFR-5/6：`confirm` 重新 canonicalize 并校验 root 仍存在且为目录；非绝对/不存在/非目录 → `confirm_failed`。
- NFR-13/AD-21：候选确认/拒绝、Source 停用核心操作键盘可达，语义化 region + 可读状态标签 + `aria-live`。
- IPC 契约（AD-9/AD-17/A-6）：`confirm`/`reject`/`disable` 返回 `Result<Envelope<Source>, ErrorEnvelope>`；`list_sources` 返回 `Envelope<Vec<Source>>`（不可失败）。Source DTO 不含 fingerprint（内部身份不外泄）。TS client 镜像同形并强校验 `api_version===API_VERSION`。
- `source_id` 格式 `src_<n>`，`n` 为 `source_registry` 自增主键（AUTOINCREMENT，删除后不复用）；路径/inode 无关，避免引入 rand。
- 锁复用既有 `IndexState { conn: std::sync::Mutex<Connection> }`；1.3 命令均为**同步**（无 `.await`），故沿用 std Mutex（async 命令改 tokio Mutex 属 1.4 既有 deferred 项）。

**Block If:**
- 需引入 Phase 0 锁定栈之外的依赖（rand / chrono / time / uuid / dirs）——用自增主键 + Unix 习惯 + std::fs/std::os::unix 解决。
- 确认逻辑需要读取记忆正文才能完成（违反 NFR-5）——1.3 仅 canonicalize 路径 + 取 metadata identity，不读目录内容。

**Never:**
- 不实现扫描/代际/索引、`scan_runs`、staging generation（1.4）；不解析/canonical 记录/`parser_version`（1.5）。
- 不实现**主动 degraded 标记 / reconcile 重发现循环 / 显式 rebind UI**——路径变化→degraded 的完整处理是 Story 4.3 的明确主题；1.3 只交付 fingerprint 身份基座 + `find_by_fingerprint` 精确匹配 + 幂等 confirm，使 4.3 可在其上构建。（"路径变化产生不同 Source、不自动合并"在 1.3 由 fingerprint 精确性保证并测试。）
- 不实现 `enumerate`/`search`/`watch`/`stable_native_ids`、`root_kind()` trait 方法（随 1.4–1.6 / 非 dir provider 增补）。
- 不为 Source 加时间戳列（SOURCE ER 实体无时间戳；最近扫描/错误时间属 1.8）。
- 不开放 localhost HTTP/WS/远程 URL；不新增网络面。`confirm`/`reject` 的入参为 CandidateSource 本身（非版本化 request envelope）——入站 envelope 反序列化延后至 ~1.6（1.1/1.2 既有 deferred 项）。
- 不删除 Source 行（无 remove 命令）；不引入 TS 测试框架（沿用 scaffold：`npm run build` 类型检查）。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| 确认新候选 | 真实存在的 Codex memories 目录候选 | canonicalize root、算 fingerprint、分配 `src_<n>`、lifecycle=`confirmed`、持久化；返回 `Envelope<Source>` | 无错误 |
| 幂等再确认 | 同一 root（同 path+inode）再次 confirm | `find_by_fingerprint` 命中既有行，返回**相同** `source_id`，不新增行 | 无错误 |
| 唤醒 rejected/disabled | 对 fingerprint 命中的 rejected/disabled 行执行 confirm | lifecycle 翻转为 `confirmed`，`source_id` 不变（confirm 即"确保 confirmed"，兼作 re-enable） | 无错误 |
| 拒绝候选 | 候选 reject | 持久化 lifecycle=`rejected` 的行；不出现在"已确认可扫描"集合 | 无错误 |
| 停用已确认 Source | 对 `confirmed` Source 的 `source_id` 执行 disable | lifecycle=`disabled`；Source 行保留；**源文件 mtime/内容不变**（NFR-1） | 无错误 |
| 确认时 root 已失效 | 候选 root 在 discover 与 confirm 间消失/变文件/非绝对 | 不写 Registry | `confirm_failed`（canonicalize/校验失败，safe message） |
| 停用未知 source_id | 不存在的 `source_id` disable | 不改 Registry | `source_not_found` |
| 路径/inode 变化 | 同 path 但 inode 变（目录重建）或 path 变（移动） | 产生**不同** fingerprint → 不同 `source_id`/不同行，不自动合并（degraded 标记本身属 4.3） | 无错误 |
| 重启保留 | 进程重启后 `list_sources` | 返回此前 confirm/reject/disable 的全部 Source（含 lifecycle） | 无错误 |
| 命令版本化 | 任一命令响应 | 携带 `api_version` 的 Envelope；失败为 ErrorEnvelope（code+message，无正文/凭据） | 形状漂移→TS client 抛 `{code:'ipc_contract'}` |

</intent-contract>

## Code Map

- `src-tauri/src/domain/source.rs` -- 新建：`SourceId`(newtype `src_<n>`)、`SourceLifecycle{Confirmed,Disabled,Rejected}`、`SourceKind{AgentMemory}`、`HealthState{Unknown,...}`、`FilesystemIdentity{device,file_id}`、`Source`（DTO，fingerprint 字段 `#[serde(skip)]`）、纯函数 `build_fingerprint(provider,root_kind,normalized_path,Option<FilesystemIdentity>) -> SourceFingerprint`。
- `src-tauri/src/domain/mod.rs` -- 加 `pub mod source;` + re-export `Source/SourceId/SourceLifecycle/SourceKind/HealthState`。
- `src-tauri/src/policy/mod.rs` -- 加 `canonicalize_root(root:&Path) -> io::Result<CanonicalRoot{normalized_path,identity}>`（`std::fs::canonicalize` + `#[cfg(unix)]` 取 `dev()/ino()`，非 unix 退 path-only）；AD-4 canonical path policy 首落地。
- `src-tauri/src/index/source_registry.rs` -- 新建：`SourceRegistry{conn:&Connection}` + 方法 `upsert_by_fingerprint`/`find_by_fingerprint`/`set_lifecycle`/`list`/`get`/`rowid<->SourceId` 映射；row↔Source 映射。
- `src-tauri/src/index/mod.rs` -- `pub mod source_registry;`。
- `src-tauri/src/index/migrations.rs` -- 追加 migration id `2` `v1_source_registry`（建 `source_registry` STRICT 表 + fingerprint 唯一索引）；`CURRENT_SCHEMA_VERSION`→2。
- `src-tauri/src/application/source.rs` -- 新建：从 `application/mod.rs` 抽出 `discover_sources`；加 `adapter_for(provider)->Option<&dyn ProviderAdapter>`（codex）、`confirm_source/reject_source/disable_source/list_sources`（编排 policy+domain+registry，按 fingerprint 幂等）。
- `src-tauri/src/application/mod.rs` -- `pub mod source;`，移除内联 `discover_sources`（改 re-export 自 `source`）。
- `src-tauri/src/ipc/envelope.rs` -- 为 ErrorEnvelope 加 `confirm_failed()`/`source_not_found()` 构造器（shape 不变，code+message）。
- `src-tauri/src/ipc/mod.rs` -- 新增 `confirm_source`/`reject_source`(CandidateSource)、`disable_source`(source_id)、`list_sources()` 命令 + wrap seam（`wrap_source`）+ 单测；错误映射 `ConfirmError`→ErrorEnvelope；保留 `discover_sources`。
- `src-tauri/src/lib.rs` -- `invoke_handler` 注册 4 个新命令；re-export。
- `src-tauri/tests/source_registry.rs` -- 新建：migration v1 + confirm/reject/disable/list + 幂等 + 唤醒 + 路径/inode 变化分离 + canonicalize symlink + NFR-1 零写入 集成测试（tempdir，不经 `env::set_var`）。
- `src/ipc/sources.ts` -- 新建：镜像 `Source`/`SourceLifecycle`/`SourceKind`/`HealthState` + `confirmSource/rejectSource/disableSource/listSources` 客户端（`api_version===API_VERSION` 强校验 + 形状守卫，漂移抛 `{code:'ipc_contract'}`）。
- `src/ipc/errors.ts` -- `TESSERA_STABLE_ERROR_CODES` 增 `confirm_failed`、`source_not_found`。
- `src/features/sources/Sources.tsx` -- 新建（取代 `features/discover/DiscoverSources.tsx`）：mount 调 `discover_sources`+`list_sources`；候选行带 Confirm/Reject 按钮；已注册 Source 区按 lifecycle 展示、confirmed 带 Disable 按钮；语义化 region + `aria-live` + 键盘可达。
- `src/features/discover/DiscoverSources.tsx` -- 删除（逻辑并入 `Sources`）。
- `src/App.tsx` -- 改为组合 `<Sources/>`（移除 DiscoverSources 导入）。

## Tasks & Acceptance

**Execution:**
- `src-tauri/src/domain/source.rs` -- 定义 `SourceId(String)`（`src_` 前缀 newtype，Display）、`SourceLifecycle`/`SourceKind`/`HealthState`（serde rename snake_case 稳定串）、`FilesystemIdentity{device:u64,file_id:u64}`、`Source{source_id,provider,source_kind,lifecycle_state,health_state,coverage_level,normalized_root_path,native_project, fingerprint(#[serde(skip)]))}`、`SourceFingerprint(String)`；纯函数 `build_fingerprint` 用 netstring 长度前缀编码（见 Design Notes） -- AD-33/AD-35 身份 + 纯函数可测。
- `src-tauri/src/policy/mod.rs` -- `canonicalize_root(root) -> io::Result<CanonicalRoot>`：`canonicalize`→normalized path；`metadata`+`#[cfg(unix)] MetadataExt` 取 `(dev,ino)`，非 unix `None`；`CanonicalRoot{normalized_path:PathBuf, identity:Option<FilesystemIdentity>}` -- AD-4 canonical path + AD-35 identity。
- `src-tauri/src/index/migrations.rs` -- 追加 `Migration{id:2,name:"v1_source_registry",apply:v1_source_registry}` 建表（`id INTEGER PRIMARY KEY AUTOINCREMENT, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project` 全 NOT NULL 除 native_project；`CREATE UNIQUE INDEX source_registry_fingerprint`）；`CURRENT_SCHEMA_VERSION=2` -- AD-29 原子 + A-19 source_kind。
- `src-tauri/src/index/source_registry.rs` -- `SourceRegistry::new(&Connection)`；`find_by_fingerprint(&str)->Option<Source>`、`upsert_by_fingerprint(Source 写入字段)->Source`（INSERT..RETURNING 或 last_insert_rowid 得 id→`src_<id>`）、`set_lifecycle(source_id,target)->Option<Source>`、`list()->Vec<Source>`、`get(source_id)->Option<Source>`；row↔Source 映射；`SourceId`/rowid 互转 -- Registry 持久化。
- `src-tauri/src/application/source.rs` -- `adapter_for("codex")->CodexAdapter`；`discover_sources()`（迁入）；`confirm_source(&SourceRegistry,&CandidateSource)->Result<Source,ConfirmError>`：canonicalize→build_fingerprint(provider,"dir",…)→`find_by_fingerprint` 命中则 `set_lifecycle(confirmed)`（唤醒）否则 `upsert`（lifecycle=confirmed, coverage 取自 adapter 而非 payload）；`reject_source`（lifecycle=rejected，对称幂等）；`disable_source(&registry,source_id)`（set_lifecycle disabled）；`list_sources(&registry)` -- AD-1 编排 + 幂等 + coverage 单一事实源。
- `src-tauri/src/application/mod.rs` -- `pub mod source;` 并 re-export `discover_sources`（移除内联体）-- 1.2 既有 "application 内联" Note 兑现。
- `src-tauri/src/ipc/envelope.rs` -- `ErrorEnvelope::confirm_failed()`（code `confirm_failed`）/`source_not_found()` 构造器 + safe message -- AD-13 shape 不变。
- `src-tauri/src/ipc/mod.rs` -- `#[tauri::command] confirm_source(candidate, state)->Result<Envelope<Source>,ErrorEnvelope>`（lock conn→`SourceRegistry::new`→`application::source::confirm_source`→`wrap_source`）；`reject_source`/`disable_source`/`list_sources` 同构；`wrap_source(Source)->Envelope<Source>` seam；单测覆盖 wrap seam + Source wire-shape 往返（fingerprint 不上线）-- A-6 版本化 + 命令薄壳。
- `src-tauri/src/lib.rs` -- `generate_handler![ping, discover_sources, confirm_source, reject_source, disable_source, list_sources]` + re-export -- 命令注册。
- `src-tauri/tests/source_registry.rs` -- 集成测试：v1 migration 建表 + schema_version=2；confirm 新候选→`src_`+confirmed+fingerprint；幂等再 confirm 同 id；reject 后 confirm 唤醒为 confirmed（同 id）；disable→disabled + **源文件 mtime/内容零变化**（NFR-1）；`find_by_fingerprint` 精确匹配；同 path 不同 inode（重建目录）→不同 fingerprint/不同 source（不合并）；canonicalize 解析 symlink（tempdir symlink）；disable 未知 id→None；list 全量 -- I/O 矩阵全行 + AD-33/35 + NFR-1。
- `src/ipc/sources.ts` -- 镜像类型 + 四个客户端（`invoke` + `api_version===API_VERSION` + 形状守卫 + 漂移抛 `TesseraIpcError{code:'ipc_contract'}`）-- A-6 TS 镜像。
- `src/ipc/errors.ts` -- stable codes 增 `confirm_failed`/`source_not_found` -- 新错误码可安全展示。
- `src/features/sources/Sources.tsx` -- mount 并发 discover+list；候选 `<ul>` 每项 Confirm/Reject 按钮（调对应 client，成功后刷新 list）；已注册 Source `<ul>` 按 lifecycle 标签展示，confirmed 项带 Disable；`<section aria-label>` + `aria-live="polite"` + 按钮键盘可达；空态保留 1.2 诚实文案（无手动添加入口）-- AC + NFR-13。
- `src/features/discover/DiscoverSources.tsx` -- 删除；`src/App.tsx` -- 组合 `<Sources/>` -- 单一 Sources 特性。

**Acceptance Criteria:**
- Given 一个真实存在的 Codex Candidate Source，when Carver 调 `confirm_source`，then 返回 `Envelope<Source>`，含 `source_id`（`src_` 前缀）、`lifecycle_state=confirmed`、`coverage_level=full`、归一化路径，且 fingerprint 已写入 Registry；源文件集合/内容/大小/mtime 不变（NFR-1）。
- Given 已确认的 Source，when 进程重启后调 `list_sources`，then 返回该 Source（lifecycle 保留 confirmed）；先前 reject/disable 的 Source 也一并保留各自 lifecycle。
- Given 同一 root 再次 `confirm_source`，when fingerprint 命中，then 返回**相同** `source_id`（幂等，不新增行）；given 早先 rejected/disabled 的 Source，when 对其候选 confirm，then lifecycle 翻转为 confirmed 且 `source_id` 不变。
- Given 一个 `confirmed` Source 的 `source_id`，when 调 `disable_source`，then `lifecycle_state=disabled`、Source 行保留、源文件不变；given 不存在的 `source_id`，when disable，then 返回 `code=source_not_found` 的 ErrorEnvelope。
- Given 候选 root 在 discover 与 confirm 之间失效（删除/变文件），when confirm，then 返回 `code=confirm_failed`；错误 message 不含正文/凭据（NFR-3）。
- Given 同路径但 inode 变化（目录被重建）或路径变化的候选，when confirm，then 产生**不同** fingerprint 与**不同** `source_id`（不与旧 Source 自动合并）。
- Given 任一命令响应，when TS client 读取，then 携带 `api_version` 的版本化 envelope 往返成功；`disable`/`list` 命令不接受任意路径（仅 `source_id`/无参），仅 `confirm`/`reject` 接受 CandidateSource（AD-4）。
- Given `cargo test`，when 运行，then source_registry 集成测试（I/O 矩阵全行 + 幂等 + 唤醒 + 路径/inode 分离 + symlink + NFR-1 零写入）通过，且 1.1/1.2 既有测试不回归。

## Spec Change Log

## Review Triage Log

### 2026-07-22 — Review pass
- intent_gap: 0
- bad_spec: 0
- patch: 2: (high 0, medium 0, low 2)
- defer: 1: (high 0, medium 0, low 1)
- reject: 0
- addressed_findings:
  - `[low]` `[patch]` `src-tauri/src/ipc/mod.rs` — 命令 Err 路径的 `map_source_error`（SourceError→ErrorEnvelope 稳定 code 映射，TS client 依赖）未被测试；新增 `map_source_error_routes_to_stable_ipc_codes` 直测三变体映射 + AD-13 安全 message。
  - `[low]` `[patch]` `src-tauri/tests/source_registry.rs` — NFR-1 零写入仅断言 disable（快照取在 confirm 之后）；新增 `confirm_and_reject_do_not_mutate_source_files_nfr1`，快照取在 confirm 之前，覆盖 confirm/reject 写路径。
  - 注：`adversarial` 与 `edge-case-hunter` 两层在本 pass 被中断（未完成）；本 pass 结论基于已完成的 `verification-gap` 与 `intent-alignment` 两层。

## Design Notes

- **source_id 方案（无 rand/uuid）：** `src_<id>`，`id` 为 `source_registry.id INTEGER PRIMARY KEY AUTOINCREMENT`。AUTOINCREMENT 保证删除后 id 不复用（"移除 Source"是未来特性，A-7）。路径/inode 无关——身份独立于 fingerprint（fingerprint 是匹配键，source_id 是稳定句柄）。`SourceId` 与 rowid 互转在 registry 内部，对外不暴露整数。
- **lifecycle 模型：** 三态 `confirmed|disabled|rejected`，全部持久化（兑现用户故事"决定在重启后保留"，含拒绝决定）。三个动作各设目标态、按 fingerprint 幂等：`confirm`=确保 confirmed（兼唤醒 rejected/disabled，免单列 re-enable 命令）；`reject`=确保 rejected；`disable`(by source_id)=确保 disabled。rebind/re-enable 之外的复杂状态转换属 4.3。
- **fingerprint 编码（纯函数、无依赖、跨版本稳定）：** netstring 长度前缀使可变串无歧义、注入安全；仅做相等比较无需解析。示例：
  ```
  // build_fingerprint("codex","dir","/Users/c/.codex/memories", Some((dev=16777231, ino=9876543)))
  // -> "root-fingerprint/v1|5:codex|3:dir|25:/Users/c/.codex/memories|i16777231:9876543"
  // identity 不可用（非 unix 或 metadata 缺失）时末段为 "n"（normalized path 显式 fallback）
  ```
  `<len>:<bytes>` 中 len 为 UTF-8 字节长；`|` 为固定分隔。相同输入→相同字节；不同输入因长度前缀不会碰撞。版本标签 `root-fingerprint/v1` 内嵌于串，未来 v2 可区分。
- **为何精确匹配 + fingerprint 唯一索引：** AD-35 "no fuzzy merge"。同 root（同 provider+path+inode）重复 confirm→`find_by_fingerprint` 命中→同一行（幂等）。path 或 inode 变→不同 fingerprint→不同行（旧 Source 保留，不合并）。degraded 的**主动标记/重发现循环/rebind UI** 是 Story 4.3 的明确主题，1.3 只提供其赖以构建的身份基座与 `find_by_fingerprint` 原语。
- **为何不加时间戳：** 架构 SOURCE ER 实体（source_id/provider/source_kind/lifecycle_state/health_state/coverage_level）无时间戳；最近扫描/最近错误时间属 1.8。延后避免引入 chrono/time（1.2 既有"非锁定栈不引依赖"先例）。
- **为何 Source DTO 隐藏 fingerprint：** fingerprint 是内部匹配键，泄漏内部身份编码无业务价值；`#[serde(skip)]` 保留内存内值、不上线。归一化路径仍展示（用户需看到真实 root）。
- **为何保留 std::sync::Mutex：** 1.3 命令同步（canonicalize + sqlite 写均同步，无 `.await`），不触 1.1 既有"async 跨 await 持锁死锁"deferred 项；该改造随首条 async DB 命令（1.4）落地。
- **为何 confirm/reject 直接收 CandidateSource 而非版本化 request envelope：** 入站 envelope 反序列化（`Envelope.api_version:&'static str` 的 Deserialize 问题）延后至 ~1.6（1.1/1.2 既有 deferred）。CandidateSource 本已是 serde 类型，作 Tauri 命令参数直传即可；响应侧版本化不变。
- **coverage 单一事实源：** 确认时 stored `coverage_level` 取自 `adapter_for(provider).coverage_level()`，不信任 payload 字段（部分缓解 1.2 deferred 的"candidate.coverage 与 trait 两份事实源"）。

## Verification

**Commands:**
- `cargo build` -- expected: 成功；domain::source + policy::canonicalize_root + index::SourceRegistry + migration v1 + 4 命令编译通过。
- `cargo test` -- expected: 通过；含 `source_registry`（migration v1 + I/O 矩阵全行 + 幂等 + 唤醒 + 路径/inode 分离 + symlink + NFR-1 零写入）+ ipc wrap_source/wire-shape + 1.1/1.2 既有全绿；schema_version=2。
- `cargo clippy --lib --test source_registry` -- expected: 无 warning。
- `npm run build` -- expected: `tsc -b && vite build` 成功（sources.ts + Sources.tsx 类型检查）。

**Manual checks (if no CLI):**
- `npm run tauri dev`：窗口 Sources 区列出 Codex 候选；点 Confirm→移入已注册列表（confirmed）；点 Disable→变 disabled；重启应用→已注册 Source 仍存在。无手动添加入口（MVP 反目标）。
- 端到端 IPC（经 invoke_handler 的 4 命令）无自动化测试——spec 以 `tauri dev` 人工检查覆盖（同 1.1/1.2 ping/discover 既有残留）；命令注册名经 `invoke_handler` 列表核对。

## Auto Run Result

Status: done

实现摘要：在 1.2 发现能力之上实现 Story 1.3 的 Source 确认/拒绝/停用与持久身份。新增 `domain::source` 身份类型（`SourceId`=src_\<rowid\>、`SourceLifecycle{confirmed,disabled,rejected}`、`SourceKind`、`HealthState`、`Source` DTO[fingerprint `#[serde(skip)]`]、纯函数 `build_fingerprint` netstring 编码 `root-fingerprint/v1`）；`policy::canonicalize_root`（canonicalize + unix `(dev,ino)` identity，非 unix path fallback）；`index::SourceRegistry`（migration id 2 `v1_source_registry` STRICT 表 + fingerprint 唯一索引 + AUTOINCREMENT）；`application::source`（confirm/reject/disable/list + `adapter_for`，按 fingerprint 幂等 + 唤醒语义，coverage 取自 adapter）；4 个版本化 Tauri command（confirm/reject 收 CandidateSource 为唯一入边界，disable 收 source_id，list 无参）；TS `sources.ts` 镜像 + `Sources.tsx` 全生命周期 UI（候选确认/拒绝、已注册停用、键盘可达）。确认状态/拒绝决定/停用均持久化跨重启。

Files changed：
- `src-tauri/src/domain/source.rs`（新）— Source 身份模型 + build_fingerprint 纯编码器
- `src-tauri/src/domain/mod.rs` — +pub mod source + re-export
- `src-tauri/src/policy/mod.rs` — +canonicalize_root（AD-4 canonical path + AD-35 identity）
- `src-tauri/src/index/source_registry.rs`（新）— SourceRegistry 持久化（find/upsert/set_lifecycle/list/get + row↔Source 映射）
- `src-tauri/src/index/mod.rs` — +pub mod source_registry，CURRENT_SCHEMA_VERSION=2
- `src-tauri/src/index/migrations.rs` — +migration id 2 v1_source_registry（STRICT 表 + 唯一索引），3 个 1.1 基线测试断言更新到 schema_version=2
- `src-tauri/src/application/source.rs`（新）— confirm/reject/disable/list + adapter_for + discover_sources（自 mod.rs 迁入）+ SourceError
- `src-tauri/src/application/mod.rs` — +pub mod source + re-export
- `src-tauri/src/ipc/envelope.rs` — +confirm_failed()/source_not_found() 构造器 + 测试
- `src-tauri/src/ipc/mod.rs` — 4 命令 + wrap_source/lock_conn/map_source_error seam + 测试（含本 pass 新增 map_source_error 映射测试）
- `src-tauri/src/lib.rs` — invoke_handler 注册 4 命令 + re-export
- `src-tauri/tests/source_registry.rs`（新）— 16 集成测试（migration v1 + I/O 矩阵全行 + 幂等 + 唤醒 + inode 分离 + symlink 折叠 + NFR-1 零写[disable/confirm/reject] + 未知 provider/id）
- `src/ipc/sources.ts`（新）— Source 类型 + 4 客户端（api_version 强校验 + 形状守卫）
- `src/ipc/errors.ts` — +confirm_failed/source_not_found 稳定 code
- `src/features/sources/Sources.tsx`（新，取代 discover）— 全生命周期 Sources UI
- `src/features/discover/DiscoverSources.tsx` — 删除（并入 Sources）
- `src/App.tsx` — 组合 `<Sources/>`

Review findings breakdown：
- patches applied：2（low 2：`map_source_error` 命令 Err 路径映射测试、NFR-1 confirm/reject 零写入断言）
- deferred：1（4 命令 invoke_handler 端到端注册/参数名约定无自动化测试——同 1.1/1.2 既有残留，spec 既定 `tauri dev` 人工检查；见 `deferred-work.md`）
- rejected：0
- 评审范围说明：4 层中 `verification-gap` 与 `intent-alignment` 完成；`adversarial` 与 `edge-case-hunter` 被中断（未完成），本 pass 结论基于已完成两层。

Follow-up review：recommended = **false**（patched 计数 high 0、medium 0、low 2；score 3×0 + 2 = 2 < 5）。

Verification performed：
- `cargo build` ✓
- `cargo test` ✓ **69 passed, 0 failed**（lib 42：envelope/migrations/ping/discover/wrap_source/wire-shape/map_source_error/domain-source；codex_discover 9；fts5 2；source_registry 16：migration v1 + I/O 矩阵全行 + 幂等 + 唤醒 + inode 分离 + symlink + NFR-1 零写[disable/confirm/reject] + 未知 provider/id + list）
- `cargo clippy --lib --test source_registry` ✓ 1.3 代码无 warning
- `npm run build` ✓（tsc -b + vite build，22 modules）
- `npm run tauri dev`（人工）— spec 既定人工检查项（GUI + 重启持久化 + 端到端 IPC），本自动化运行未执行。

Residual risks：
- 端到端 IPC（经 invoke_handler 的 4 命令）与 JS/Rust 参数名约定（source_id↔sourceId）无自动化测试（同 1.1/1.2 残留；见 deferred）。
- 跨进程重启持久化为结构性测试（同库写读回 + migration 幂等）覆盖，非进程重启测试；spec AC 由 `tauri dev` 人工检查覆盖。
- `adversarial`/`edge-case-hunter` 两评审层未完成（被中断），未被该两层覆盖的潜在缺陷风险存在；建议后续可补跑这两层。
- 出站网络仅配置层验证，无运行时/CI 门禁（同 Phase 0 残留）。
- 无 VCS：baseline/final_revision = NO_VCS；变更未提交（项目无 git 仓库）。
- coverage 双事实源（1.2 deferred）仅部分缓解（confirm 取 adapter coverage，但 CandidateSource payload 仍自带字段）；完整解决属 Epic 2。
- 主动 degraded 标记/reconcile/rebind UI 显式延后至 Story 4.3；1.3 只交付 fingerprint 身份基座 + find_by_fingerprint 精确匹配。

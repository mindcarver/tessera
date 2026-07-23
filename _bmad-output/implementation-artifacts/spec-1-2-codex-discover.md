---
title: 'Codex Candidate Source 自动发现与展示'
type: 'feature'
created: '2026-07-21'
status: 'done'
baseline_revision: 'NO_VCS'
final_revision: 'NO_VCS'
review_loop_iteration: 0
followup_review_recommended: true
context:
  - '_bmad-output/implementation-artifacts/epic-1-context.md'
  - '_bmad-output/planning-artifacts/architecture/architecture-tessera-2026-07-20/ARCHITECTURE-SPINE.md'
  - '_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md'
warnings:
  - oversized
---

<intent-contract>

## Intent

**Problem:** Story 1.1 搭好了 Tauri/Rust 骨架与 `ping` 契约样本，但 `ProviderAdapter` 仍是空 trait、Codex adapter 是空模块、UI 无任何 Source 信息——Carver 启动后看不到本机有哪些可接入的 Codex Agent Memory 来源。

**Approach:** 实现 `ProviderAdapter` 的**发现切片**（`discover` + `coverage_level` + `CoverageLevel` 枚举）与 Candidate Source 元数据类型；Codex adapter 解析 `CODEX_HOME/memories` 或 `~/.codex/memories` 的**目录存在性**并产出候选（不读内容）；application 层编排发现；新增版本化、无参的 `discover_sources` IPC command；UI 启动时调用并列出候选或显示无候选空态（不提供手动添加目录入口）。本 Story 仅"看见"，不做确认/扫描/解析。

## Boundaries & Constraints

**Always:**
- AD-1：Rust core 是唯一边界。发现经 application service → adapter；UI 只调用已登记的 typed Tauri command（`discover_sources`），不直接访问文件系统/Provider/SQLite。
- AD-3 / A-3：Codex adapter 声明 `provider_id`、`coverage_level`、`discover`。`CoverageLevel` 取值 `full | search_only | existence_only | unsupported`；Codex 为本地可完整枚举存储，声明 `full`（描述 Provider 性质，`enumerate` 的实现属 1.5）。
- AD-4：`discover` 只产出 Candidate Source **元数据**，不持久化、不 canonicalize root、不分配 `source_id`（确认与 fingerprint 持久身份属 1.3）。
- AD-11 / NFR-5：发现阶段只检查 memories **目录是否存在**（元数据级），不读取任何聊天/transcript/正文内容。
- AD-12 / NFR-2：发现无任何出站网络；错误信封（AD-13）为 stable code + safe message，不含正文/凭据。
- NFR-13：候选列表为语义化结构（`region`/`list` + 可读 `aria-label`），核心信息键盘可达。
- IPC 契约（AD-9/AD-17/A-6）：`discover_sources` 返回 `Envelope<Vec<CandidateSource>>`，携带 `api_version`；TS client 镜像同形。
- `CODEX_HOME` 优先级：若 `CODEX_HOME` 非空，只查 `$CODEX_HOME/memories`（**不**回退 `~/.codex`）；否则查 `$HOME/.codex/memories`。无候选（0 个）不是错误——返回空 vec，UI 渲染空态。

**Block If:**
- 发现逻辑需要读取文件内容或枚举文件集才能完成（违反 NFR-5 / 超出 1.2 范围）——本 Story 仅目录存在性检查。
- 需引入 Phase 0 锁定栈之外的依赖（如 `dirs`/`chrono`）——用 `std::env` 解析路径；候选不带 `observed_at`。

**Never:**
- 不实现确认/拒绝/停用、`source_id`/fingerprint 持久化（1.3）。
- 不实现扫描/代际/索引、`scan_runs`（1.4）；不解析/canonical 记录/`parser_version`（1.5）。
- 不实现 `enumerate`/`search`/`watch`/`stable_native_ids` 的具体行为——trait 在 1.2 仅含发现切片，其余方法随 1.4–1.6 增补到 trait（`provider_adapter.rs` Phase 0 doc 既定）。
- 不提供手动添加任意目录入口（AC2 / MVP 明确反目标）；不读取或渲染任何记忆正文。
- 不在 `discover_sources` 之外新增 IPC command；不新增网络/HTTP/WS 面。
- 不引入 vitest/jest 等 TS 测试框架（沿用 scaffold：TS 经 `npm run build` 类型检查）。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| 默认目录存在 | `$HOME/.codex/memories` 存在，`CODEX_HOME` 未设 | `discover` 返回 1 候选：provider=`codex`、root_path=`$HOME/.codex/memories`、basis=`default_home`、coverage=`full`、native_project=None；UI 列出该候选 | 无错误（`discover` 不可失败） |
| CODEX_HOME 显式指向 | `CODEX_HOME=/x` 非空且 `/x/memories` 存在 | 返回 1 候选，basis=`codex_home_env`、root_path=`/x/memories`，**不**回退 `~/.codex` | 无错误 |
| 无受支持来源 | 无 `CODEX_HOME/memories` 亦无 `~/.codex/memories` | 返回空 vec；UI 显示空态且无手动添加入口 | 不是错误 |
| CODEX_HOME 设但 memories 缺失 | `CODEX_HOME=/x` 非空但 `/x/memories` 不存在 | 返回空 vec（不回退 `~/.codex`） | 不是错误 |
| 目录仅含被排除工件 | memories 目录存在但仅含 transcript/JSONL 样文件 | `discover` 仍返回 1 候选（仅检目录存在，不读内容、不解析、不区分文件类型） | 无错误（NFR-5 由构造保证） |

</intent-contract>

## Code Map

- `src-tauri/src/domain/ports/provider_adapter.rs` -- ProviderAdapter trait 发现切片（`provider_id` 已存；新增 `coverage_level` + `discover`）+ `CoverageLevel` 枚举 + `DiscoveryBasis` 枚举 + `CandidateSource` 元数据类型；更新模块 doc 标注发现切片在 1.2 落地、其余方法随 1.4–1.6 增补。
- `src-tauri/src/adapters/codex.rs` -- `CodexAdapter`（unit struct）实现 trait：`provider_id="codex"`、`coverage_level=Full`、`discover` 按 CODEX_HOME 优先级解析 memories 目录存在性，存在产 1 候选否则空 vec；仅 `std::env`+`std::path`，不读内容。**路径解析抽为纯函数** `resolve_codex_memories_root(codex_home: Option<&str>, home: Option<&str>) -> Option<(DiscoveryBasis, PathBuf)>`（注入参数、不读 env），`discover()` 读 env 后调它再 `Path::exists()`。
- `src-tauri/src/application/mod.rs` -- application 层 `discover_sources()`：编排 CodexAdapter discover（无持久化、无 canonicalize），返回 `Vec<CandidateSource>`。
- `src-tauri/src/ipc/mod.rs` -- 新增 `discover_sources()` command 返回 `Envelope<Vec<CandidateSource>>`；不可失败故无错误分支。
- `src-tauri/src/lib.rs` -- `invoke_handler` 注册 `discover_sources`。
- `src-tauri/src/domain/mod.rs` -- re-export `CandidateSource`/`CoverageLevel`/`DiscoveryBasis`。
- `src/ipc/discover.ts` -- typed TS client 镜像 `CandidateSource` + `Envelope`；`invoke('discover_sources')`；形状漂移抛 `{code:'ipc_contract'}`。
- `src/features/discover/DiscoverSources.tsx` -- 启动调用 discover，渲染候选列表（provider/path/basis/native_project）或空态（无手动添加入口）；语义化 region/list + 可读标签。
- `src/App.tsx` -- 组合 `DiscoverSources`（保留既有 ping 段）。
- `src-tauri/tests/codex_discover.rs` -- discover 契约/coverage_level/env 优先级/NFR-5（tempfile fixture）测试。

## Tasks & Acceptance

**Execution:**
- `src-tauri/src/domain/ports/provider_adapter.rs` -- 定义 `CoverageLevel`（serde rename 到 stable 串 `full`/`search_only`/`existence_only`/`unsupported`）、`DiscoveryBasis`（rename `default_home`/`codex_home_env`）、`CandidateSource`（`provider: String`、`root_path: String`、`basis: DiscoveryBasis`、`coverage_level: CoverageLevel`、`native_project: Option<String>`，均 `Serialize`+`Deserialize`）；扩展 trait 加 `coverage_level(&self) -> CoverageLevel` 与 `discover(&self) -> Vec<CandidateSource>`（不可失败）；更新模块 doc -- A-3 契约 + AD-3 coverage_level + AD-4 候选元数据。
- `src-tauri/src/adapters/codex.rs` -- `CodexAdapter`：`provider_id()=="codex"`、`coverage_level()==Full`、`discover()` 解析 `CODEX_HOME`（非空则 `$CODEX_HOME/memories`，否则 `$HOME/.codex/memories`），`Path::exists()` 为真产 1 候选，否则空 vec -- A-3 声明 + NFR-5 + Codex 边界（AD-11）。
- `src-tauri/src/application/mod.rs` -- `pub fn discover_sources() -> Vec<CandidateSource>`：构造 `CodexAdapter` 调 `discover()`（无状态/无持久化；1.3 再抽 `source` 子模块与 registry） -- AD-1 唯一边界。
- `src-tauri/src/ipc/mod.rs`,`src-tauri/src/lib.rs` -- 新增 `#[tauri::command] pub fn discover_sources() -> Envelope<Vec<CandidateSource>>` 并登记进 `invoke_handler` -- A-6 IPC 契约 + AD-17 版本化。
- `src/ipc/discover.ts` -- typed client：镜像 `CandidateSource`/`Envelope`，`invoke<Envelope<CandidateSource[]>>('discover_sources')`，形状校验失败抛 `{code:'ipc_contract'}` -- A-6 TS 镜像。
- `src/features/discover/DiscoverSources.tsx`,`src/App.tsx` -- 组件启动调 discover，按状态机渲染候选列表（每项 provider/path/basis/native_project）或空态（**无**手动添加入口）；`<section aria-label>`+`<ul>`+可读文本；App 组合该组件并保留 ping 段 -- AC1/AC2 + NFR-13。
- `src-tauri/tests/codex_discover.rs` -- 契约测试（**不**用 `std::env::set_var`——并行竞争/edition-2024 不安全；改注入 tempdir 路径到纯解析器与存在性 helper）：默认参数+目录存在→1 候选（`default_home`）；`codex_home` 非空+目录存在→1 候选（`codex_home_env`）且不回退；无目录→空；`codex_home` 设但 memories 缺失→空；memories 仅含 `.jsonl` 样文件仍返回候选且测试不读其内容（NFR-5）；`coverage_level()==Full` -- A-3/AD-14 capability-honesty 前置 + I/O 矩阵覆盖。

**Acceptance Criteria:**
- Given 本机有 `~/.codex/memories`（或 `CODEX_HOME/memories`），when 应用启动发现，then UI 列出 1 个 Codex Candidate Source，显示 Provider=`codex`、候选路径、发现依据；发现未读取任何聊天/transcript 内容（NFR-5），Codex adapter 声明 `discover` + `coverage_level`（A-3）。
- Given 本机无受支持 Codex 来源，when 启动发现，then UI 显示空态且**不**提供手动添加目录入口。
- Given `discover_sources` 响应，when UI 读取，then 携带 `api_version` 的版本化 envelope 往返成功（UI→core→UI）。
- Given `cargo test`，when 运行，then discover 契约 / coverage_level / env 优先级 / NFR-5 测试通过，且 1.1 既有测试不回归。

## Spec Change Log

## Review Triage Log

### 2026-07-21 — Review pass
- intent_gap: 0
- bad_spec: 0
- patch: 11: (high 0, medium 2, low 9)
- defer: 5: (high 0, medium 0, low 5)
- reject: 8: (high 0, medium 0, low 8)
- addressed_findings:
  - `[medium]` `[patch]` `src-tauri/src/adapters/codex.rs`,`src-tauri/tests/codex_discover.rs` — `discover()` 三步胶水未被验证（7/8 测试用 test-local helper 镜像逻辑，从不调用 adapter）。新增 `discover_with_env` + `candidate_if_existing_dir` seam，矩阵测试改为驱动真实 adapter 路径（tempdir 注入），删除 helper 镜像。
  - `[medium]` `[patch]` `src/features/discover/DiscoverSources.tsx` — UI 文案 "fully enumerable" 过度承诺（enumerate 属 1.5）。改为诚实描述当前切片的文案。
  - `[low]` `[patch]` `src-tauri/src/adapters/codex.rs` — `Path::exists()` 改 `is_dir()`：memories 路径为普通文件或 broken symlink 时不报告为候选。
  - `[low]` `[patch]` `src-tauri/src/ipc/mod.rs` — IPC 测试在无候选主机上 vacuous；抽 `wrap_discover` seam 用注入载荷非空验证，原 command 调用改为 infallibility smoke。
  - `[low]` `[patch]` `src-tauri/src/adapters/codex.rs` — root 路径非 UTF-8 时丢弃候选（避免 `to_string_lossy` 的 U+FFFD 显示路径）。
  - `[low]` `[patch]` `src/ipc/errors.ts`(新),`src/App.tsx`,`src/features/discover/DiscoverSources.tsx` — 重复的 `readTesseraErrorMessage`/`TESSERA_STABLE_ERROR_CODES` 抽至共享 `src/ipc/errors.ts`。
  - `[low]` `[patch]` `src/features/discover/DiscoverSources.tsx` — 删除 `<ul>` 上与 `<section>` 重复的 `aria-label`（屏阅读器双重播报）。
  - `[low]` `[patch]` `src/ipc/discover.ts` — 强制 `envelope.api_version === API_VERSION`（原仅 `typeof` 校验；AD-17 版本化生效）。
  - `[low]` `[patch]` `src/ipc/discover.ts` — 移除 `asCandidateSource` 中不可达的 `?? null` dead code。
  - `[low]` `[patch]` `src-tauri/src/adapters/codex.rs` resolver — 空白-only `CODEX_HOME`/`HOME` 视为未设（trim）。
  - `[low]` `[patch]` `src-tauri/src/adapters/codex.rs` resolver — 拒绝相对 `CODEX_HOME`/`HOME`（避免 CWD 相关、跨启动漂移的候选）。

## Design Notes

- **`discover` 不可失败（返回 `Vec`，非 `Result`）：** 发现仅做 `Path::exists()`（infallible，错误→false）；pre-confirm 阶段无 `source_id` 可挂 AD-13 的 source-scoped 错误；"无法 stat"诚实地即"无候选"。故无需 domain 级 error 类型，也避免 domain→ipc 反向依赖（`ErrorEnvelope` 现位于 `ipc::envelope`）。错误路径随 1.4/1.5 真正可失败的 scan/parse 落地。
- **CoverageLevel：** 枚举 `Full`/`SearchOnly`/`ExistenceOnly`/`Unsupported`，serde rename 到 stable 串。Codex=Full（本地可完整枚举，描述 Provider 性质；`enumerate` 实现属 1.5，此时声明 Full 仍诚实）。
- **候选不带 `observed_at`：** AC1 展示列表无此项；候选每次启动重新发现为临时态；时间戳属持久 Source/record（1.3/1.5）——避免引入 `chrono`/日期格式化（非锁定栈）。
- **路径解析仅 `std::env`（`HOME`/`CODEX_HOME`）：** 不引 `dirs` crate（非锁定栈；1.1 review 曾拒未锁定依赖）。`CODEX_HOME` 为空串视为未设。
- **Native project：** Codex memories 为全局存储、无可发现的按项目划分 → `native_project=None`（"可判定"指可判定时才显示；Codex 不可判定）。1.5 解析阶段若能从内容推断再补结构化类型。
- **trait 增量生长：** 1.2 仅 `provider_id`/`coverage_level`/`discover`；`enumerate`/`search`/`watch`/`stable_native_ids` 随各自 Story 增补（`provider_adapter.rs` Phase 0 doc 既定，非投机式预留）。
- **1.2 不引入入站 request envelope：** `discover_sources` 无参（同 `ping`），故 `deferred-work.md` 中 `Envelope.api_version: &'static str` 的反序列化项保持延后，至首条带版本化请求载荷的 command（约 1.6 查询）再处理。
- **application 内联：** 1.2 把 `discover_sources` 内联于 `application/mod.rs`（无状态）；1.3 引入 confirm/registry 时再抽 `application/source.rs`。
- **env 可测试性：** `CODEX_HOME`/`HOME` 依赖的路径解析抽为纯函数（注入参数），测试以 tempdir 路径注入，**不**经 `std::env::set_var`（`cargo test` 并行下竞争，且 edition 2024 标 `unsafe`）。`discover()` 仅做"读 env → 纯解析 → `Path::exists()`"三步胶水。

## Verification

**Commands:**
- `cargo build` -- expected: 成功；trait 发现切片 + `CodexAdapter` + `discover_sources` command 编译通过。
- `cargo test` -- expected: 通过；含 `codex_discover`（契约/coverage/env 优先级/NFR-5）+ ipc `discover_sources` envelope 测试 + 1.1 既有测试全绿。
- `npm run build` -- expected: `tsc -b && vite build` 成功（TS client + `DiscoverSources` 类型检查）。
- `npm run tauri dev`（人工）-- expected: 窗口打开，发现段列出 Codex 候选或空态，无手动添加入口。

**Manual checks (if no CLI):**
- 若本机无 `~/.codex/memories`：临时 `mkdir -p ~/.codex/memories` 启动验证候选出现，移除后验证空态；设 `CODEX_HOME=/tmp/x` + `mkdir -p /tmp/x/memories` 验证 `codex_home_env` basis 与不回退。

## Auto Run Result

Status: done

实现摘要：在 1.1 脚手架之上实现 Story 1.2 的 Codex Candidate Source 自动发现与展示。`ProviderAdapter` trait 增加发现切片（`coverage_level` + `discover`）与 `CoverageLevel`/`DiscoveryBasis`/`CandidateSource` 类型；`CodexAdapter` 按 `CODEX_HOME` 优先级解析 `CODEX_HOME/memories` 或 `~/.codex/memories` 的目录存在性（`is_dir`、仅元数据、NFR-5）并产出候选（不读内容、不持久化、不分配 source_id、不 canonicalize）；application 层 `discover_sources()` 编排；新增版本化、无参、不可失败的 `discover_sources` IPC command（`Envelope<Vec<CandidateSource>>`，经 `wrap_discover` seam 便于非空测试）；TS client `discover.ts` 镜像并强制 `api_version===API_VERSION`；`DiscoverSources` 组件启动调用、列出候选或空态（无手动添加入口）、诚实 coverage 文案。错误消息 helper 抽至共享 `src/ipc/errors.ts`。

Files changed：
- `src-tauri/src/domain/ports/provider_adapter.rs` — CoverageLevel/DiscoveryBasis/CandidateSource + trait 发现切片（provider_id/coverage_level/discover）
- `src-tauri/src/domain/mod.rs` — re-export 新类型
- `src-tauri/src/adapters/codex.rs` — CodexAdapter（discover / discover_with_env seam / candidate_if_existing_dir + 纯解析器 resolve_codex_memories_root，含 is_dir、绝对路径、trim、非 UTF-8 守卫）
- `src-tauri/src/application/mod.rs` — discover_sources() 编排
- `src-tauri/src/ipc/mod.rs` — discover_sources command + wrap_discover seam + 单元测试
- `src-tauri/src/lib.rs` — 注册 discover_sources 进 invoke_handler
- `src/ipc/discover.ts` — typed TS client（api_version 强校验、形状守卫）
- `src/ipc/errors.ts`（新）— 共享 readTesseraErrorMessage / TESSERA_STABLE_ERROR_CODES
- `src/features/discover/DiscoverSources.tsx`（新）— 候选列表 / 空态 / 诚实 coverage 文案 / 共享错误 helper
- `src/App.tsx` — 组合 DiscoverSources + 使用共享错误 helper（移除本地副本）
- `src-tauri/tests/codex_discover.rs`（新）— 9 集成测试（I/O 矩阵全行 + is_dir + capability + glue smoke）

Review findings breakdown：
- patches applied：11（medium 2：discover seam + 矩阵测试真实化、UI 诚实文案；low 9：is_dir、IPC wrap_discover seam、非 UTF-8 守卫、errors.ts 去重、aria-label 去重、api_version 强校验、dead code、trim、绝对路径守卫）
- deferred：5（见 `deferred-work.md`：coverage 双源真相[Epic 2]、Windows USERPROFILE[Phase A macOS-only]、discover 超时[同 ping-timeout 残留]、invoke_handler 注册测试[同 1.1 残留]、ping.ts api_version[1.1 既有]）
- rejected：8（路径 `.`-segment 规范化[理论、近 1.3 canonicalize]、plain-object throw[1.1 既有约定、Phase A 无遥测]、IPCMarker/claude_code 桩[1.1 既有/Epic 2 占位]、broken-symlink 区分诊断[1.8 health 范围、空态已正确]、error 重试按钮[错误为非瞬态契约漂移]、UI 回环未测[spec 既定 Never + accessibility.spec.ts 占位 + 人工检查]、重复 capability 测试[无害]、native-project 歧义[spec 已选可辩护读法]）

Follow-up review：recommended = **true**（patched 计数 high 0、medium 2、low 9；score 3×2 + 9 = 15 ≥ 5）。

Verification performed：
- `cargo build` ✓
- `cargo test` ✓ **30 passed, 0 failed**（lib 19：envelope/migrations/ping/discover_sources/wrap_discover/wire-shape；codex_discover 9：矩阵 5 行 + is_dir + capability + resolver paths + glue smoke；fts5 2）
- `cargo clippy --lib --test codex_discover` ✓ 无 warning
- `npm run build` ✓（tsc -b + vite build，21 modules，`dist/` 产出）
- `npm run tauri dev`（人工）— spec 既定人工检查项（GUI），本自动化运行未执行；command 已注册（smoke spawn 确认）。

Residual risks：
- 端到端 IPC（经 invoke_handler 的 discover_sources）无自动化测试（spec 以 `npm run tauri dev` 人工检查覆盖；同 Phase 0 ping 残留）。
- 出站网络仅配置层验证，无运行时/CI 门禁（同 Phase 0 残留）。
- discover() 无超时（NFS/慢 FS 场景；Phase A 本地不触发，见 deferred）。
- 无 VCS：baseline/final_revision = NO_VCS；变更未提交（项目无 git 仓库）。
- 其余前瞻项见 `deferred-work.md`。

---
title: '本地应用骨架与可启动运行（Phase 0 脚手架）'
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
warnings:
  - oversized
---

<intent-contract>

## Intent

**Problem:** 仓库当前是全新空仓库（无 `package.json`/`Cargo.toml`/`src`/`src-tauri`）。后续所有功能（发现 Codex 记忆、确认、扫描、搜索、打开）都需要一个可运行、离线、无账号、以 Rust core 为唯一应用边界的 shell 才能构建。

**Approach:** 用 Phase 0 锁定的技术栈与结构种子 bootstrap 一个最小可启动的 Tauri 2 桌面应用骨架：Rust core 七模块骨架 + React 19/Vite 8 shell + rusqlite(bundled, FTS5) + notify；提供带 `api_version` 的 typed IPC ping 往返；建立 migration 框架（v0）；预留基准与可访问性占位；完成 A-15 Deferred 验证（FTS5 中文 tokenizer / CSP-Markdown sanitizer / 外部 SQLite 只读模式）并记录结论。本 Story 是 Epic 1 其余所有 Story 的基础。

## Boundaries & Constraints

**Always:**
- 栈与版本由 Phase 0 锁定，**精确到 minor**：Rust stable `1.97.x`、Tauri `2.x`、React `19.2.7`、Vite `8.1.x`、rusqlite `0.40.1`（`bundled` feature，FTS5 enabled）、SQLite `3.x`（经 rusqlite bundled）、notify `8.2.x`。**精确 patch**（Rust patch、Tauri/Tauri CLI/`tauri-build` patch、前端依赖 patch）在 bootstrap 时解析为当前可用版本并写入 `rust-toolchain.toml`、`Cargo.lock`、`package-lock.json`/lockfile。
- 结构种子固定且不可改名：Rust core `src-tauri/src/{domain, application, adapters, index, state, policy, ipc}`；UI `src/{features, components, ipc}`；测试与产出 `tests/ui/accessibility.spec.ts`、`tests/benchmarks/memory-index.json`；Provider fixture `src-tauri/tests/fixtures/providers/{codex, claude_code}`。
- Rust core 是唯一应用边界（AD-1）：所有文件访问/解析/索引/查询须经 core application service；UI 只能调用已登记的 typed Tauri command；UI 不直接依赖 Provider、文件系统或 SQLite。
- IPC 契约（AD-9/AD-17/A-6）：请求-响应用带 `api_version` 的 typed Tauri Commands；本 Story 仅实现 `ping` 一个 command 作为契约样本。
- Local-only（AD-12/AD-20/A-14/NFR-2）：启动过程无任何出站网络请求；MVP 无出站网络路径、无账号、无遥测、无自动更新；日志默认 omit 正文/查询词/凭据（本 Story 暂无正文，仅确立默认脱敏约定）。
- migration 原子执行，失败保留旧 index（AD-29/A-7）；v0 仅建立框架与 meta（如 `schema_version`/已应用 migration 记录），完整业务 schema 留给后续 Story（1.4/1.5）。
- 基准占位（A-16/NFR-11）：`tests/benchmarks/memory-index.json` 的阈值/baseline **必须留空**，不得在基准建立前编造任何固定数值。
- Phase 0 A-15 Deferred 项须**先验证再决定**实现路径，不得用未验证的便利实现绕过（见 Tasks 的验证文档）。

**Block If:**
- 上述锁定的 minor 版本无法解析为可构建配置（如 Rust `1.97.x`、Tauri `2.x`、rusqlite `0.40.1` bundled + FTS5 任一在 bootstrap 时无法取得/构建）。minor 偏离需更新架构，不可自行换版。
- `bundled` rusqlite 无法启用 FTS5（`CREATE VIRTUAL TABLE ... USING fts5` 不可用），且无法通过 feature flag/编译选项修复。
- 启动时观察到出站网络请求且无法通过 Tauri 配置（禁 updater、CSP `default-src 'self'`、禁远程）消除。
- Phase 0 验证发现某 A-15 Deferred 项在锁定栈上不可行（如 FTS5 对中文短查询完全不可用、外部 SQLite 只读模式不可行），需提升为新 AD 或调整栈。

**Never:**
- 不实现业务功能（发现/确认/扫描/解析/搜索/打开）——只搭骨架与契约样本。
- 不实现完整 SQLite 业务 schema、FTS5 搜索 schema、scan_runs 状态机、Provider adapter 逻辑（留给 1.2–1.6）。
- 不做签名、公证、自动更新、跨平台 installer、远程服务、localhost HTTP/WS、MCP/CLI（AD-20，Deferred）。
- 不接入 Codex/Claude Code 真实目录的读写逻辑；不读取任何 Agent Memory 正文（本 Story 仅建框架）。
- 不编造性能阈值；不在 ping 之外新增 IPC command；不引入未在 Phase 0 锁定的额外依赖（除构建必需的 dev-tooling）。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Ping 往返（happy） | UI 启动后调用 `ping` command | core 返回带 `api_version` 的 typed envelope，UI 渲染响应（如展示版本/pong） | 无错误预期 |
| Ping 版本契约（error 样本） | 调用方携带不兼容/缺失 `api_version` 的请求 | core 返回结构化错误信封（stable `code` + safe `message`），不 panic | 错误信封不含正文/凭据（AD-13 约定，本 Story 仅确立形态） |

</intent-contract>

## Code Map

- `rust-toolchain.toml` -- 锁定 Rust stable 1.97.x exact patch（bootstrap 解析）。
- `src-tauri/Cargo.toml` -- tauri 2.x（exact patch）、rusqlite 0.40.1 `bundled`、notify 8.2.x、serde 等；FTS5 经 bundled 启用。
- `src-tauri/tauri.conf.json` -- Tauri 2 配置：无 updater、CSP `default-src 'self'`、无远程、macOS 目标。
- `src-tauri/src/{domain,application,adapters,index,state,policy,ipc}/mod.rs` -- 七模块骨架（每个模块声明存在、导出占位类型/trait 位置，不实现业务）。
- `src-tauri/src/lib.rs` / `src-tauri/src/main.rs` -- Tauri 入口：注册 `ping` command、启动时跑 migration v0。
- `src-tauri/src/ipc/mod.rs`（含 ping） -- typed `ping` command + 版本化 envelope + 错误信封形态。
- `src-tauri/src/index/mod.rs` + `src-tauri/src/index/migrations.rs` + migration v0 资源 -- migration runner（原子、失败保留旧）+ v0 meta。
- `package.json` / `vite.config.ts` / `tsconfig.json` -- React 19.2.7 + Vite 8.1.x 前端栈。
- `src/main.tsx` / `src/App.tsx` / `src/ipc/ping.ts` -- React shell 调用 ping 并渲染；typed TS client 镜像 envelope。
- `tests/ui/accessibility.spec.ts` -- 可访问性 spec 占位（窗口可打开、ping 可键盘触达的 smoke）。
- `tests/benchmarks/memory-index.json` -- 基准占位，阈值/baseline 留空。
- `src-tauri/tests/fixtures/providers/{codex,claude_code}/.gitkeep` -- fixture 目录种子。
- `src-tauri/tests/fts5_available.rs`（或合并入 index 模块测试） -- 断言 FTS5 虚表可创建。
- `docs/phase-0-verification.md` -- A-15 Deferred 验证结论记录，作为 1.5/1.6 实现路径与是否提升为新 AD 的依据。

## Tasks & Acceptance

**Execution:**
- `rust-toolchain.toml` -- 锁定 Rust stable 1.97.x exact patch（bootstrap 解析当前可用 patch） -- A-1 toolchain 锁定。
- `src-tauri/Cargo.toml` -- 声明 tauri 2.x（exact patch，含 Tauri CLI/`tauri-build` patch）、rusqlite 0.40.1 `bundled`、notify 8.2.x、serde；FTS5 经 bundled 启用 -- A-1 栈锁定 + FTS5。
- `package.json`,`vite.config.ts`,`tsconfig.json` -- React 19.2.7 + Vite 8.1.x shell + Tauri TS 集成；lockfile 持精确 patch -- A-1 前端栈。
- `src-tauri/tauri.conf.json` -- Tauri 2 配置：禁 updater、CSP `default-src 'self'`（禁远程脚本/raw HTML/script/event handler/js URL）、macOS 目标、无远程端点 -- A-14/A-15 local-only。
- `src-tauri/src/{domain,application,adapters,index,state,policy,ipc}/mod.rs`,`src-tauri/src/lib.rs`,`src-tauri/src/main.rs` -- 七模块骨架（声明模块、占位核心 trait/类型位置如 `domain/ports`、`adapters`、`state`、`policy`、不写业务逻辑）+ Tauri 入口注册 ping、启动跑 migration v0 -- A-1 结构种子 + A-2 唯一边界。
- `src-tauri/src/ipc/mod.rs` -- typed `ping` command：返回版本化 envelope（含 `api_version`）；定义结构化错误信封形态（stable code + safe message） -- A-6 IPC 契约 + AD-13 错误信封。
- `src-tauri/src/index/migrations.rs`,migration v0 资源 -- migration runner：原子应用、失败保留旧；v0 建 meta（已应用 migration 记录/schema_version） -- A-7 migration 框架 v0。
- `src/main.tsx`,`src/App.tsx`,`src/ipc/ping.ts` -- React shell 启动调用 ping、渲染带 `api_version` 的响应；typed TS client 镜像 Rust envelope -- AC IPC 往返 + A-6。
- `src-tauri/tests/fts5_available.rs` -- 测试 `CREATE VIRTUAL TABLE t USING fts5(...)` 成功 -- AC "FTS5 可用"。
- `src-tauri/tests/fixtures/providers/{codex,claude_code}/.gitkeep` -- 创建 fixture 目录种子（空） -- A-13/A-18 锚点。
- `tests/ui/accessibility.spec.ts` -- 可访问性 smoke 占位：窗口打开 + ping 路径键盘可达（最低覆盖项，后续 Story 扩充） -- A-17。
- `tests/benchmarks/memory-index.json` -- 占位结构（cold scan/query/memory/index-size 四项字段），所有阈值与 baseline 值留空/null -- A-16/NFR-11 不编造阈值。
- `docs/phase-0-verification.md` -- 记录 A-15 Deferred 验证结论：(1) FTS5 中文 tokenizer `trigram` vs `unicode61` 在真实 Codex 中文样本上的召回/空结果率/短查询延迟/索引体积对比与推荐；(2) Markdown/Agent Memory 不可信内容 CSP + sanitizer 方案（`default-src 'self'`、禁远程脚本、禁 raw HTML/script/event handler/js URL）；(3) 外部 SQLite `mode=ro` + WAL sidecar 可行性（禁 `immutable=1`/`nolock=1`）；(4) exact toolchain build check 结论。每项给出"是否需提升为新 AD"的判断 -- AC + readiness m1 闭环。

**Acceptance Criteria:**
- Given 一台 macOS 本机，when Carver 构建（`cargo build` + `npm run build`）并启动应用，then Tauri 2 + Rust core（七模块骨架）+ React 19/Vite 8 shell + rusqlite(bundled, FTS5) + notify 就绪并打开窗口。
- Given 应用已启动，when UI 调用 `ping`，then 一个带 `api_version` 的 typed IPC 响应 UI→core→UI 往返并在 UI 渲染。
- Given 启动过程，when 监听网络，then 无任何出站网络请求（NFR-2）；`rust-toolchain.toml` 锁定 stable patch。
- Given 仓库，when 运行 `cargo test` 与 `npm run build`，then 二者通过；FTS5 可用测试通过；migration 框架就绪（v0 可原子应用）。
- Given 仓库，when 检查产出路径，then `tests/benchmarks/memory-index.json`（阈值留空）与 `tests/ui/accessibility.spec.ts` 占位存在。
- Given Phase 0 验证，when 阅读 `docs/phase-0-verification.md`，then FTS5 中文 tokenizer、CSP/sanitizer、外部 SQLite 只读模式、toolchain build check 四项均有结论与"是否提升为新 AD"判断，作为 1.5/1.6 实现路径依据。

## Design Notes

**版本化 IPC envelope（契约样本，illustrative）：**
```rust
// 版本化响应信封 —— ping 仅作为契约样本，确立后续所有 command 共用形态
pub struct Envelope<T> { pub api_version: &'static str /* 如 "1" */, pub payload: T }
#[tauri::command]
pub fn ping() -> Envelope<Pong> { Envelope { api_version: API_VERSION, payload: Pong { .. } } }
```
TS 侧 `src/ipc/ping.ts` 镜像同一 envelope 形状。`api_version` 为契约主版本（string，如 `"1"`）；具体取值由 dev 选定但必须出现且 typed。

**exact patch 在 bootstrap 解析：** minor 由架构锁定（不可偏离）；patch 取 bootstrap 时当前可用版本写入 lockfile 与 `rust-toolchain.toml`。若 minor 不可解析，命中 Block If。

**Phase 0 验证方法学：** FTS5 tokenizer 对比须用 Carver 真实 Codex 中文记忆样本（脱敏/本地），度量召回/空结果率/短查询延迟/索引体积，**记录数字而非提前设阈值**；CSP/sanitizer 给出可落地配置而非仅原则；外部 SQLite 只读模式验证是否被 bundled rusqlite 支持（本 Story 用 bundled，只读模式结论面向后续可能的只读访问场景）。结论写入 `docs/phase-0-verification.md`，并在文档顶部标注"若提升为新 AD，需更新 ARCHITECTURE-SPINE"。

## Verification

**Commands:**
- `cargo build` -- expected: 成功，Tauri 2 + 七模块骨架 + rusqlite bundled 编译通过。
- `cargo test` -- expected: 通过，含 FTS5 可用测试与 migration v0 应用测试。
- `npm run build` -- expected: React 19/Vite 8 前端构建成功。
- `npm run tauri dev`（或 `cargo tauri dev`）手动/CI 启动 -- expected: 窗口打开，ping 往返在 UI 可见。
- 网络出站检查（如启动时用 `lsof`/抓包/防火墙日志） -- expected: 无出站连接。

**Manual checks (if no CLI):**
- 启动应用观察窗口渲染 ping 响应；确认 `rust-toolchain.toml`、`Cargo.lock`、前端 lockfile 含精确 patch；确认 `tests/benchmarks/memory-index.json` 无编造阈值；确认 `docs/phase-0-verification.md` 四项结论齐全。

## Review Triage Log

### 2026-07-21 — Review pass
- intent_gap: 0
- bad_spec: 0
- patch: 10: (high 2, medium 2, low 6)
- defer: 10: (high 0, medium 1, low 9)
- reject: 5: (high 0, medium 0, low 5)
- addressed_findings:
  - `[high]` `[patch]` `src/ipc/ping.ts` — Tauri 2 `invoke` 返回值即命令返回值；原解构 `const {api_version, ...payload}` 产生嵌套 `{payload:{...}}`，破坏 ping 往返契约。改为 `invoke<Envelope<Pong>>` 直接返回并按 `api_version`+`payload.name/version` 校验。
  - `[high]` `[patch]` `src/ipc/ping.ts` — 形状不匹配时不再 fabricate `{name:"tessera",version:"unknown"}` 假成功，改为抛 `code:"ipc_contract"` 结构化错误让 UI 渲染 error 态（Phase 0 目的是证明往返，不可静默伪造）。
  - `[medium]` `[patch]` `.gitignore` — 原忽略 `src-tauri/Cargo.lock`/`Cargo.lock`，违反 spec Always「exact patch 经 Cargo.lock 锁定」。移除两行并注释说明二进制 crate 应提交 lockfile。
  - `[medium]` `[patch]` `docs/phase-0-verification.md` — FTS5 中文 tokenizer 结论补 Block If #4 判定（FTS5 可用、非完全不可用、未触发）+ Story 1.6 硬门禁（真实 fixture 短查询召回非零方可锁定，否则升级 AD）。
  - `[low]` `[patch]` `src-tauri/src/index/migrations.rs` — `iso_now_utc` 名/文档称 RFC 3339 实返 Unix 秒、且时钟失败 `.unwrap_or(0)` 写 "0"。重命名 `unix_seconds_now`、订正文档、时钟失败写 "unknown" sentinel。
  - `[low]` `[patch]` `src-tauri/src/index/migrations.rs` — `schema_version` 存在但不可解析时不再 `parse().unwrap_or(0)` 静默重置为 0 重跑全部 migration；区分「行缺失→0」与「行存在但不可解析→Err」。
  - `[low]` `[patch]` `docs/phase-0-verification.md` — `style-src 'unsafe-inline'` 改述为「仅因 Phase 0 无不可信内容而可接受」，1.5 须剥离 style；补 `ipc:`/`http://ipc.localhost` 为 Tauri 内部 scheme。
  - `[low]` `[patch]` `src-tauri/tests/fts5_available.rs` — trigram 测试原先只建虚表不验证 CJK 分词，补 ≥3 字符 CJK 插入 + MATCH 断言，使「pin 能力」名副其实。
  - `[low]` `[patch]` `src/App.tsx` — `readTesseraErrorMessage` 原把任意 `err.message` 写入 DOM；改为仅在 `code` 属已知 Tessera stable code（internal/ipc_contract）时采纳，否则用通用串（AD-12/AD-13 脱敏）。
  - `[low]` `[patch]` `src-tauri/Cargo.toml` — 移除未使用且不在 Phase 0 锁定栈的 `thiserror`、`log`（notify 为 spec 锁定栈保留，opener 已在 lib.rs 注册）。

## Auto Run Result

Status: done

实现摘要：从空仓库 bootstrap Phase 0 脚手架。Tauri 2 + Rust 1.97.0（exact patch 锁 `rust-toolchain.toml`/`Cargo.lock`）+ React 19.2.7 / Vite 8.1.0 + rusqlite 0.40.1（bundled, FTS5）+ notify 8.2。Rust core 七模块骨架 + 版本化 IPC `ping`（`Envelope<T>` 带 `api_version`）+ 原子 migration 框架（v0_meta）+ React shell 渲染 ping + FTS5/trigram 可用测试 + 基准/可访问性占位 + Provider fixture 种子 + Phase 0 A-15 验证文档。

Files changed（绿色新增，baseline NO_VCS 无前序代码）：
- 根：`rust-toolchain.toml`、`.gitignore`、`package.json`、`package-lock.json`、`vite.config.ts`、`tsconfig.{json,app,node}.json`、`index.html`
- `src-tauri/`：`Cargo.toml`、`Cargo.lock`、`build.rs`、`tauri.conf.json`、`capabilities/default.json`
- `src-tauri/src/`：`lib.rs`、`main.rs`、`domain/{mod,ports/{mod,provider_adapter,index_store,query_store}}`、`application/mod`、`adapters/{mod,codex,claude_code}`、`index/{mod,migrations}`、`state/mod`、`policy/mod`、`ipc/{mod,envelope}`
- `src-tauri/tests/`：`fts5_available.rs`、`fixtures/providers/{codex,claude_code}/.gitkeep`
- `src/`：`main.tsx`、`App.tsx`、`ipc/ping.ts`、`features/.gitkeep`、`components/.gitkeep`
- `tests/`：`ui/accessibility.spec.ts`、`benchmarks/memory-index.json`
- `docs/phase-0-verification.md`

Review findings breakdown：
- patches applied：10（high 2：ping.ts 契约 bug + fabricate；medium 2：gitignored Cargo.lock、FTS5 结论/1.6 门禁；low 6：iso_now_utc、schema_version 损坏、inline-CSS doc、trigram CJK 测试、错误消息脱敏、未用依赖）
- deferred：10（见 `deferred-work.md`；主要为 Story 1.4+ 前瞻项：boot 容错、async Mutex、降级守卫、端到端 IPC 测试、capability 收紧、运行时网络门禁等）
- rejected：5（rust-version exact pin 为 spec「exact patch at bootstrap」所要求；accessibility 占位 AC 已满足（仅要求占位存在）；I/O 矩阵 row2 错误 producer 形态已交付（ErrorEnvelope + 脱敏测试），实际 producer 需可失败命令、正确延后；audit `INSERT OR REPLACE` 被 schema_version 修复覆盖（重跑路径不可达）；intent-alignment 表面差异为 spec 既定验证面：人工 `tauri dev` / 配置层）

Follow-up review：recommended = **true**（patched high=2；含 high 即 true；score 3×medium + low = 3×2 + 6 = 12 ≥ 5）。patched 计数：high 2、medium 2、low 6。

Verification performed：
- `cargo build` ✓
- `cargo test` ✓ **10 passed**（envelope ×3、migrations ×4 含原子回滚、ping ×1、FTS5 ×2 含新增 CJK trigram round-trip）
- `npm run build` ✓（`dist/` 产出）
- 启动 / 出站网络：配置层验证（无 updater/远程/telemetry；CSP `default-src 'self'`；capabilities 仅 `core:default` 不含 FS/shell/HTTP；vite 绑 127.0.0.1）；运行时 lsof/抓包 + `npm run tauri dev` 为 spec 既定人工检查项（GUI，未在本自动化运行执行）。

Residual risks：
- ping 端到端（经 `invoke_handler` 注册）无自动化测试（spec 以 `npm run tauri dev` 人工检查覆盖）；ping.ts 契约已修正但运行时尚未实跑验证。
- 出站网络仅配置层验证，无运行时/CI dep-scan 门禁。
- FTS5 中文短查询召回为已知风险，硬门禁落在 Story 1.6（见 `docs/phase-0-verification.md`）。
- 无 VCS：baseline/final_revision = NO_VCS；`Cargo.lock` 已从 `.gitignore` 移除（待入库时提交），但当前无 git 仓库可提交。
- 评审 verification-gap 层因网关 529 三次未返回；其覆盖面已由 adversarial + edge-case + intent-alignment 三层 + 父 agent 自评补足（网络运行时核验、ping 端到端、FTS5 可运行背书、migration replay vs apply 等均已覆盖）。
- 其余前瞻项见 `deferred-work.md`。

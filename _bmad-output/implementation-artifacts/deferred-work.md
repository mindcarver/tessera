# Deferred Work

Append-only collection of review findings deferred for later focused attention.
Each entry: source spec, one-sentence summary, why it is real.

- source_spec: `_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md`
  summary: boot 路径（`src-tauri/src/lib.rs::run`）对 app_data_dir / create_dir_all / Connection::open / migrations::apply 全部 `.expect()`，启动期 I/O 或 migration 失败直接 panic，未走 AD-13 结构化错误信封。
  evidence: 真实但属前瞻——Phase 0 为全新 DB、v0_meta 幂等，实际不会失败；migration 原子性本身已测（`failed_migration_batch_rolls_back_atomically`）。优雅启动错误处理归属 Story 1.4（migration 失败/恢复为该 Story 主题）。review adversarial#4 / edge#1-3。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md`
  summary: `invoke_handler` 注册的 `ping` 未经 Tauri 命令机制测试（单测直接调函数）；注册名拼错可编译通过且全测过、仅 `tauri dev` 暴露。
  evidence: `ipc/mod.rs` 单测 `ping_returns_versioned_envelope` 直接调 `ping()`，不经 invoke。自动化端到端 IPC 测试（`tauri::test` 或 Playwright）延后；spec Verification 以 `npm run tauri dev` 人工检查覆盖。review adversarial#18。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md`
  summary: `Envelope<T>.api_version: &'static str` 的 Deserialize 在运行时 `serde_json::from_str` 不可用（仅 static-lending deserializer）；当前仅序列化响应故可编译，未来 request envelope 反序列化会断。
  evidence: `ipc/envelope.rs:18-24` derive Deserialize 但 Phase 0 无入站反序列化。改为 `String`/`Cow` 当请求 envelope 在 Story 1.2 落地时处理。review adversarial#9。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md`
  summary: `capabilities/default.json` 授予 `core:default`（含 window/menu/tray/path 等），广于「仅 invoke 已注册命令」；`core:default` 不含 FS/shell/HTTP（AD-1 未违），但 threat model 成熟后应收紧。
  evidence: Phase 0 无不可信内容，`core:default` 为 Tauri 推荐最小集；收紧需确认 invoke 所需最小权限以免破坏窗口操作。归属 Story 1.5（渲染不可信 Markdown 时）。review adversarial#7。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md`
  summary: migration runner 对 `schema_version` 高于 max migration id（旧二进制对新 DB，降级）静默 Ok。
  evidence: `index/migrations.rs::apply` 仅过滤 `id > current`，未检测 `current > max`。Phase 0 单 migration 使其学术化；多 migration 出现时（Story 1.4）加降级守卫。review edge#6。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md`
  summary: `ensure_meta_tables` 在 apply 事务外执行，第二批 CREATE 若失败 meta 半建成无回滚。
  evidence: `index/migrations.rs`。`CREATE TABLE IF NOT EXISTS` 幂等，失败罕见且下次启动自愈；框架硬化归属 Story 1.4。review edge#8。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md`
  summary: `App.tsx` `useEffect` 的 ping 无超时；`invoke('ping')` 永不 resolve 时 UI 永久 loading。
  evidence: `src/App.tsx:25-45`。Phase 0 ping 为本地、快速；超时为打磨项。review edge#13。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md`
  summary: `IndexState` 用 `std::sync::Mutex<Connection>`，未来 async 命令跨 `.await` 持锁会死锁 WebView invoke。
  evidence: `src-tauri/src/lib.rs`。Phase 0 ping 不触 state；首条触 DB 的 async 命令（Story 1.4）前改 `tokio::sync::Mutex` 或 `spawn_blocking`。review edge#4。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md`
  summary: migration 测试全部用 `Connection::open_in_memory`；on-disk boot 路径（`lib.rs` `Connection::open`）未测，WAL/回滚差异可能隐藏 boot-only 失败。
  evidence: `index/migrations.rs` 测试 + `lib.rs`。migration 逻辑与存储无关；on-disk 文件 I/O 经 boot / `tauri dev` 人工覆盖。补 tempfile 集成测试为可选增强。review edge#15。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md`
  summary: spec AC「启动无出站网络」仅配置层验证，无运行时 lsof/抓包或 CI dep-scan 门禁；`http://ipc.localhost` 的 local-only 为假设。
  evidence: 配置层确无远程端点（CSP/capabilities/feature），但无自动化守卫捕获未来误加 `reqwest`/`fetch`。运行时核验为 spec 既定人工检查项；CI dep-scan 为后续可加门禁。review adversarial#15。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-2-codex-discover.md`
  summary: `CandidateSource.coverage_level` 字段与 `ProviderAdapter::coverage_level()` 是两份事实源，struct 可被任意构造；第二个 adapter（Claude Code，Epic 2）可能推送与其 trait 声明不一致的 coverage，而 UI 信任 per-candidate 字段。
  evidence: 真实但仅在第二个 provider 落地时有后果。当前单 Codex provider 二者一致。Epic 2 引入 Claude Code 时加固（private 构造器或 candidate.coverage==adapter.coverage 不变量测试）。review adversarial#5。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-2-codex-discover.md`
  summary: `discover()` 只读 `HOME`，未读 Windows 的 `USERPROFILE`；Windows 主机会看到空态。
  evidence: Phase A 明确仅 Carver 当前 macOS 单机（AD-20），Windows 不在范围。跨平台重访时加 `USERPROFILE` 回退。review adversarial#6。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-2-codex-discover.md`
  summary: `discover()` 的 `is_dir()` 与 UI invoke 均无超时；网络挂载（NFS/FUSE）上的 `HOME` 可阻塞 stat，React 停留 loading。
  evidence: Phase A 为 Carver 本地 Mac（HOME 本地、快速），场景不触发；与 1.1 既延后的 ping 无超时同类。网络/慢 FS 纳入范围时为 discover 设延迟预算 + UI 超时。review adversarial#8 / edge#6。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-2-codex-discover.md`
  summary: `discover_sources` 在 `invoke_handler` 的注册未经测试（拼写错误可编译过、全测过、仅运行时暴露）。
  evidence: 同 1.1 既延后项（ping 注册未测）。加 `const &[&str]` 命令名 + 断言，或 Playwright 激活时覆盖。review verification-gap#2。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-2-codex-discover.md`
  summary: `src/ipc/ping.ts` 的 `api_version` 仅 `typeof` 校验、未与 `API_VERSION` 常量比较（本 pass 已在 `discover.ts` 修复的同一问题）。
  evidence: 1.1 既有代码（ping.ts），非 1.2 引入。下次触碰 ping.ts 时加同一行 `envelope.api_version === API_VERSION` 校验以保持一致。review adversarial#11。
- source_spec: `_bmad-output/implementation-artifacts/spec-1-3-source-confirm.md`
  summary: 4 个新 Tauri 命令（confirm_source/reject_source/disable_source/list_sources）经 `invoke_handler` 的端到端注册与 JS/Rust 参数名约定（`source_id`↔`sourceId` camelCase 默认绑定）无自动化测试。
  evidence: 同 1.1 ping / 1.2 discover 既有残留；spec Verification 既定 `tauri dev` 人工检查覆盖。命令薄壳经 lib 单测（wrap_source/wire-shape/map_source_error）覆盖，注册名拼写错误或 Tauri 参数名漂移仅运行时暴露。review verification-gap#1。


## 2026-07-22 传输改造处置（sprint-change-proposal-2026-07-22）

Tauri 移除、传输改为 loopback-only HTTP（tiny_http）后，上述条目处置如下：

**已关闭：**

- boot `.expect()` panic 项 —— `server/src/lib.rs::boot` 改为返回 `io::Result`；`main.rs` 打印错误并 `exit(1)`，启动失败不再 panic。
- ping/discover/confirm 等「invoke_handler 注册未经测试」三项（1.1 ping、1.2 discover、1.3 四命令 + camelCase 绑定）—— Tauri invoke 机制已不存在；路由为字面量字符串、请求 DTO 为显式 snake_case serde struct，且新增 wire 级集成测试 `server/tests/http_api.rs` 通过真实 socket 命中 ping/discover/scan 路由，路由拼写错误会使测试失败。
- `capabilities/default.json` 权限收紧项 —— Tauri capabilities 已随壳删除；浏览器 UI 的能力边界由 AD-4（仅接受 `source_id`/`record_id`）+ AD-9（Host/Origin 校验）承载。
- 「async 命令跨 await 持锁死锁」项 —— 新传输为同步设计：tiny_http 每连接一线程、handler 阻塞执行，`std::sync::Mutex<Connection>` + 同步 handler 成为既定模式（Story 1.4 spec 模式原样成立），不再规划 async 命令。
- 「migration 测试仅 in-memory、on-disk boot 未测」项 —— `http_api.rs` 测试经 `boot()` 在 tempfile scratch 目录上跑真实 on-disk boot（migrations + recover_scans）。
- `ping.ts` `api_version` 仅 typeof 校验项 —— 已改为 `envelope.api_version === API_VERSION` 比较（本 pass）。

**仍然开放（与传输无关，原样保留）：**

- `Envelope<T>.api_version: &'static str` 反序列化限制 —— 请求 envelope 反序列化落地时（~1.6）改 `String`/`Cow`。
- migration runner 降级守卫（`schema_version` 高于 max id 静默 Ok）。
- `ensure_meta_tables` 在 apply 事务外执行。
- `App.tsx` ping 无超时 —— fetch 同样无超时；loopback 本地调用保持快速路径，超时仍属打磨项。
- 「启动无出站网络」运行时/CI 门禁 —— 配置层已无任何远程端点（CSP `connect-src 'self'`、无 ipc.localhost、依赖树无 HTTP client）；运行时 lsof 核验已于 2026-07-22 人工执行（见 docs/phase-0-verification.md），CI dep-scan 门禁仍为后续可选项。
- `CandidateSource.coverage_level` 双事实源（Epic 2 引入 Claude Code 时加固）。
- `discover()` 仅读 `HOME` 未读 `USERPROFILE`（跨平台重访时处理；app-data 路径已改由 dirs crate 解析）。
- `discover()` `is_dir()` 无超时（慢/网络文件系统纳入范围时处理）。

**新增延后项：**

- 扫描进度 SSE（递增 sequence + cancellation）随 Story 1.8 落地；当前手动重扫为请求-响应式 `POST /api/scan` + `GET /api/scan/status` 轮询。
- 端口冲突时无自动换端口：bind 失败即报错退出（Phase A 单机可接受）；`TESSERA_PORT` 环境变量可手工改端口。

## Deferred from: code review of spec-1-4-scan-pipeline (2026-07-22)

- 读时 containment 重校验失败接到 `fail_run` 而非跳过该文件（`server/src/application/scan.rs:184-190`）——spec Block If 字面语义是"逃逸文件跳过不索引"；需真实并发改名竞态才可触发，与本次 review decision 2（读时 realpath vs 枚举 manifest 粒度不对齐）同根因，随其修订一并处理。
- boot 回收测试仅覆盖 `staging` 一种 stale 状态、单 source（`server/tests/scan_pipeline.rs:532-553`）——`queued/running/committing` 回收路径与多 source GC 隔离无测试；回收逻辑本身已被单路径真实执行验证，属测试加固。
- 损坏的 `tessera_meta.active_generation`（指向不存在的 generation）被静默报 0 记录（`server/src/index/scan_store.rs:414-425`）——正常路径不可达（CAS 单事务写入 + GC 保留 active，仅外部 DB 编辑可产生），但与同模块 `latest_run` 的"损坏要响亮"原则不一致；4.x 索引重建主题落地时一并处理。

## Deferred from: code review of spec-2-1-claude-discover (pass 2, 2026-07-25)

- source_spec: `_bmad-output/implementation-artifacts/spec-2-1-claude-discover.md`
  summary: `ClaudeCodeAdapter::enumerate_*` hard-fails with `EnumerateError::Unreadable`, whose documented meaning is "a directory inside the root could not be read" — a misleading diagnostic if the scan guard is ever bypassed.
  evidence: The loud-fail itself holds (a misrouted scan errors rather than silently indexing), so this is diagnostic-only. `EnumerateError` has no semantically-correct "provider unsupported" variant today. Revisit when Story 2.2 implements real Claude enumeration (the enumerate path will be replaced then).
- source_spec: `_bmad-output/implementation-artifacts/spec-2-1-claude-discover.md`
  summary: On the reserved-rescan path, `store.fail_run(...)` for `ProviderNotScannable` discards its `Result` (`let _ =`), so a SQLite write failure could leave the run row non-terminal and let boot recovery re-label it `stale_recovered`, overriding the provider-aware inventory message.
  evidence: `server/src/application/scan.rs` reserved guard. The `let _ =` pattern matches the file's existing convention at other sites; the `stale_recovered` override only triggers on a DB write failure during `fail_run` (narrow). Low severity, consistent with current style.
- source_spec: `_bmad-output/implementation-artifacts/spec-2-1-claude-discover.md`
  summary: Sync `POST /api/scan` on a `claude_code` source leaves no persistent trace (no `scan_runs` row), so the inventory `latest_error` only reflects rescan failures, not sync-scan failures — an unobvious asymmetry.
  evidence: The sync guard fires before `begin_run` by design (spec scopes the inventory surface to "when a rescan is triggered"). The asymmetry is undocumented. Document it, or have the sync guard write a failed row if cross-entry-point consistency is later desired.

## Deferred from: code review of spec-2-2-claude-parse-index (2026-07-25)

- source_spec: `_bmad-output/implementation-artifacts/spec-2-2-claude-parse-index.md`
  summary: The Phase 0 perf gate (`server/tests/performance_baseline.rs::phase_zero_baseline_gate_measures_and_enforces_the_approved_fixture`) uses tight, machine-calibrated thresholds (cold_scan ≤ 12ms on a 6ms baseline, etc.) that can false-fail under parallel `cargo test` load or on slower/dev machines — making the "no perf regression" claim clock-dependent.
  evidence: Pre-existing test infra (Story 1.9), NOT introduced or modified by 2.2. On a reviewer machine it failed 3/3 in isolation (21–42ms vs 12ms); under parallel load it flakes. The clock-independent "no Codex behavioral regression" is already proven by `codex_canonicalization` (parser-output pin) and the cross-coexistence dispatch test. Harden by widening the threshold to absorb runner variance (e.g. 5–10× baseline), marking the gate advisory, pinning it to a reference CI runner with its own baseline, and/or adding a Codex record_id/body golden-master snapshot for clock-independent behavioral regression.

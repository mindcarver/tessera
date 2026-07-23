---
title: '只读扫描管线与原子代际切换（骨架）'
type: 'feature'
created: '2026-07-22'
status: 'done'
baseline_revision: 'NO_VCS'
final_revision: 'NO_VCS'
review_loop_iteration: 2
followup_review_recommended: false
context:
  - '_bmad-output/implementation-artifacts/epic-1-context.md'
  - '_bmad-output/planning-artifacts/architecture/architecture-tessera-2026-07-20/ARCHITECTURE-SPINE.md'
  - '_bmad-output/implementation-artifacts/spec-1-3-source-confirm.md'
warnings:
  - oversized
---

<intent-contract>

## Intent

**Problem:** Story 1.3 让 Carver 能确认 Source，但确认后什么都没有发生——没有扫描、没有索引、没有"哪些记录当前可见"的概念。Epic 1 要求扫描以"完整成功才可见、失败保留上一成功版本"的方式建立索引，且永远看不到半套或失败覆盖的结果（NFR-9）。

**Approach:** 建立只读扫描管线骨架：migration v2 新增 `scan_runs`（状态机 + 持久单调 fencing token）与 `memory_records`（generation 标记的基线 file-level 记录，AD-30）两张 STRICT 表；`application::scan` 编排"快照 manifest → 写 staging generation → commit 前最终校验 → 单事务 CAS 切换 active generation"；进程启动回收 stale run（AD-16）；`dirty_after_validation` 的 generation 永不激活（AD-36）。UI 获得最小可见面：confirmed Source 上的 Scan 按钮 + 最近一次扫描结果标签。

## Boundaries & Constraints

**Always:**
- AD-1：扫描编排只经 `application::scan`；UI 只调已登记 typed command；Adapter 只读 Source 文件。
- AD-5/AD-28/AD-32：每个 Source 单一扫描 owner。fencing token 持久、每 Source 单调递增，与 generation intent 一起存 `scan_runs`；commit 在同一事务内 CAS（`UPDATE scan_runs SET state='succeeded' WHERE scan_id=? AND state='committing' AND fencing_token=?`，0 行=失败），CAS 成功的同事务内才写 `tessera_meta.active_generation` 并把 run 标记 succeeded。
- AD-34：扫描开始先对 Source root 建 manifest（relative path + size + mtime 快照）；commit_cas 之前重校验 manifest（`snapshot-at-validation` 一致性级别）；boundary 变化 → run 置 `failed` 且 `scan_runs.error_code='dirty_after_validation'`（该列即 AD-36 的持久标记槽位），其 generation 永不激活。
- AD-36：`dirty_after_validation` generation 永不切换为 active；下次扫描即是有界 retry（1.4 不自动重试——见 Never）。
- AD-16：进程启动（boot，migrations 之后）把 stale `queued/running/staging/committing` run 置为 `failed`（回收），并删除**全部非 active generation** 的 `memory_records` 行（stale in-flight 与历史 failed 的 staging 一并 GC）；保留上一 active generation 及其记录。
- NFR-1/SM-2：扫描对 Source 零写入；测试快照扫描前后文件集合/内容/size/mtime 断言不变。
- AD-4/NFR-5/6：扫描只接受 `source_id`；只枚举 Confirmed Source 的 canonical root 之内；每个文件读取前重校验仍在 root 内（`starts_with` 于 canonical root）；root 读失败 → `confirm_failed`。
- AD-11：只枚举 Supported Artifact Matrix 的 Codex 记忆工件（`MEMORY.md`、`memory_summary.md`、`raw_memories.md`、`rollout_summaries/*.md`）；未知文件跳过不索引（`unsupported_artifact` 诊断的计数属 1.5，本 Story 只需不索引）。
- AD-30/AD-15：file-level unit 基线——`unit_kind='file'`、`native_unit_id` = root 相对路径、`native_locator` = file URI、`record_id` = `rec_<hex>` 由 `source_id + provider + native_locator + unit_kind` 经无依赖 FNV-1a hash 稳定生成；content hash（同 FNV-1a over bytes）只用于变化检测。不宣称 section identity。
- AD-13：失败用结构化 ErrorEnvelope；新增 stable code `scan_failed`（复用 `internal`/`source_not_found`/`confirm_failed`）；错误不含正文/凭据；一 Source 失败不影响其他 Source generation。
- IPC（AD-9/AD-17/A-6）：`scan_source(source_id)` 同步返回 `Result<Envelope<ScanOutcome>, ErrorEnvelope>`（`ScanOutcome{source_id, scan_id, generation, records_indexed}`，仅计数/代际——无正文；失败走 Err(ErrorEnvelope)，故 Ok 即成功，DTO 不带冗余 outcome 字段）；`get_scan_status(source_id)` 返回 `ScanStatus{source_id, state, active_generation, active_records}`（最近一次 run 的 state + 当前 active generation + active 记录数）。TS client 镜像同形并强校验 `api_version===API_VERSION`。
- 锁：扫描命令同步执行（同 1.3 模式），在 `IndexState` std Mutex 临界区内完成全部扫描+commit——串行化天然保证单一 owner；**不**改 async/不引入 tokio（见 Never）。async 改造仍是 deferred 项。
- NFR-13/AD-21：Scan 按钮键盘可达；扫描结果经 `aria-live` 播报。

**Block If:**
- 发现确认过的 root 在扫描时 symlink 逃逸出 canonical root（文件 realpath 不在 root 内）——该文件跳过（不索引）；若 root 本身不可解析则整个扫描 `scan_failed`。无需人工介入，但若 Carver 的真实 `~/.codex/memories` 含逃逸 symlink 导致大量跳过，HALT 报告。
- 需要读记忆正文正文内容来“验证解析正确性”——本 Story 只存 content hash 不存正文；不要为此加正文列（1.5 主题）。

**Never:**
- 不解析 heading/section、不存正文/标题、不建 FTS5、不实现搜索/查询（1.5/1.6）；不加 `parser_version` 列之外的解析语义（1.4 恒写 `file-level/v1`）。
- 不实现 watcher/notify 接线、reconcile 循环、自动定期扫描、scan cancellation、进度 Channel（AD-8/AD-9 的这些切片属 1.8/4.x）。
- 不实现自动 retry 调度：`retry` 状态值存在于枚举但 1.4 不写入（有界 retry = Carver 手动再扫）。
- 不实现被动 degraded 标记/重发现/rebind（4.3）。
- 不改写 `std::sync::Mutex` 为 tokio、不把扫描做成 async/后台线程（保持 1.3 既定同步命令模式；deferred 项原样保留）。
- 不触碰 Source 文件（零写入）；不开放网络面；不新增锁定栈外依赖（hash 用自实现 FNV-1a，不引 `sha2`/`blake3`/`rand`/`chrono`）。
- 不为 Source/scan_runs 加 RFC 3339 时间戳列（1.3 Design Notes 既定决策：最近扫描时间属 1.8）；`scan_runs.finished_at` 用 Unix 秒整数字段复用 `migrations::unix_seconds_now` 风格。
- 不实现 Reset Index / 移除 Source 的派生清理（AD-29 的该切片随对应命令落地）。
- 不实现 UI 上的记录列表/正文预览（1.6 面）；`get_scan_status` 只暴露 state + active generation + 计数。

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| 首次扫描成功 | Confirmed Codex Source，root 含 2 个记忆文件 | run 经 queued→running→staging→committing→succeeded；staging generation（如 `gen_1`）在 commit 事务 CAS 后成为 active；`memory_records` 有 2 行 file-level 记录；返回 `Envelope<ScanOutcome{records_indexed:2, generation}>` | 无错误 |
| 扫描中途失败 | 扫描中某文件读失败（权限/消失） | run 置 `failed`；staging generation 行保留但**不激活**；上一 active generation（若有）保持 active、记录完整可见；返回 `scan_failed` ErrorEnvelope | `scan_failed`（safe message，无正文/路径细节泄漏之外的诊断） |
| 扫描期间源变化 | commit 前最终 manifest 校验发现 size/mtime/文件集合变化 | run 置 `failed` 且 `error_code='dirty_after_validation'`；active generation 不变；该 staging 记录永不激活（下次 boot GC） | `scan_failed`（code 相同，message 区分"源已变化"） |
| commit CAS 竞争 | 模拟：commit 时 `scan_runs` 当前 token/state 与持有者不符 | CAS `UPDATE` 影响 0 行 → 整个 commit 事务回滚；active generation 不变；run 保持 `committing`（重启回收为 failed） | `scan_failed` |
| 崩溃恢复 | 进程在 run 处于 `running/staging/committing` 时退出，重启 | boot 回收：stale run → `failed`；全部非 active generation 的 `memory_records` 行删除（含历史 failed run 的 staging）；上一 active generation 及记录保留可见 | 无错误（恢复是启动常态） |
| 重复扫描幂等 | 对已有 active generation 的 Source 再次扫描（源未变） | 新 generation（单调递增）写 staging→commit→CAS 切换为新 active；旧 generation 记录被清理（AD-2 派生数据可重建）；`record_id` 集合稳定（locator-based） | 无错误 |
| 空目录扫描 | Confirmed Source 的 root 无任何记忆工件 | 扫描成功，generation 激活，`records_indexed:0`；空也是完整成功（诚实状态） | 无错误 |
| 未知/未确认 Source | `scan_source` 传未知 `source_id` 或 rejected/disabled Source | 不写任何 scan 行 | `source_not_found` / `scan_failed`（message 指明非 confirmed） |
| root 已失效 | 确认后 root 被删除/变文件 | 扫描失败；active generation（若有）保留；Source 行不动（degraded 标记属 4.3） | `confirm_failed`（复用：root 校验失败） |
| 零写入 | 任意成功/失败扫描 | 扫描前后 Source 文件集合/内容/size/mtime 逐文件断言不变（SM-2） | 无错误 |
| 只枚举边界 | root 含 `rollout_summaries/*.md`（允许）+ `sessions/foo.jsonl` + `CLAUDE.md`（排除） | 只有矩阵内文件进 manifest/索引；JSONL/规则文件不索引 | 无错误 |

</intent-contract>

## Code Map

- `src-tauri/src/index/migrations.rs` -- 追加 migration id `3` `v2_scan_generations`（`scan_runs` + `memory_records` STRICT 表 + 索引）；`CURRENT_SCHEMA_VERSION`→3；既有测试断言 schema_version=2 处更新为 3。
- `src-tauri/src/domain/scan.rs` -- 新建：`ScanRunState`（`queued/running/staging/committing/succeeded/failed/retry` serde snake_case）、`Generation`（newtype `gen_<n>`）、`ScanOutcome` DTO、`ScanStatus` DTO、`ScanError`；纯函数 `fnv1a_hex(bytes)->String` 与 `build_record_id(source_id,provider,native_locator,unit_kind)`。
- `src-tauri/src/domain/mod.rs` -- `pub mod scan;` + re-export。
- `src-tauri/src/domain/ports/provider_adapter.rs` -- ProviderAdapter trait 追加 `enumerate_file_units(&self, root:&Path) -> Result<Vec<FileUnit>, EnumerateError>`（Codex 矩阵边界内枚举，纯枚举不读正文；错误类型由实现定义，不要返回 `Result<_,()>`）；`FileUnit{relative_path, absolute_path, size, mtime}`，其中 `mtime` 必须保留**亚秒精度**（如 nanos/micros，i64），禁止整秒截断（AD-34 边界校验对同秒内改写必须可检测）。
- `src-tauri/src/adapters/codex.rs` -- 实现 `enumerate_file_units`：walk `MEMORY.md`/`memory_summary.md`/`raw_memories.md`/`rollout_summaries/*.md`（一层已知文件名 + rollout_summaries 目录一层 glob；拒绝 symlink 逃逸、非 UTF-8、root 外路径）；不读正文。
- `src-tauri/src/index/scan_store.rs` -- 新建：`ScanStore{conn:&Connection}`：`begin_run(source_id)->(scan_id,fencing_token,generation)`（同事务：token=max(token)+1 per source、插 queued 行）；`set_state`；`stage_records`（**plain INSERT**，composite PK 下不做任何覆盖）；`commit_cas(scan_id,token,generation)->bool`（单事务：CAS UPDATE 的 WHERE 必须含 `state='committing'` **且** `fencing_token = (SELECT MAX(fencing_token) FROM scan_runs WHERE source_id=?)`——对**每 Source 最新 token** 比较，0 行=回滚返回 false；成功才写 `tessera_meta.active_generation` + 清理旧 generation + 标记 succeeded）；`fail_run(scan_id, error_code: &str)`（**必填**稳定类别词汇，见 Design Notes）；`recover_stale_runs()`（boot 用，stale→failed 且 `error_code='stale_recovered'`）；`active_generation(source_id)`；`latest_run(source_id)`；`count_active_records(source_id)`。`latest_run` 对无法解析的持久化 state 字符串**不得**静默映射为 failed——返回错误（未知 state = 数据损坏，表面化为 Internal）。
- `src-tauri/src/index/mod.rs` -- `pub mod scan_store;`；`CURRENT_SCHEMA_VERSION=3`。
- `src-tauri/src/application/scan.rs` -- 新建：`scan_source(registry,&Connection,source_id)->Result<ScanOutcome,ScanError>` 编排：校验 confirmed+root → **begin_run**（先建行，初始 manifest_revision 占位）→ adapter 枚举建 manifest + UPDATE 真实 revision → 逐文件读 bytes 算 content hash + 组 file-level 记录（staged）→ 最终 manifest 重校验（AD-34/36）→ commit_cas；begin_run 之后任何失败都 `fail_run`（CAS 失败除外，见 Design Notes）并返回结构化错误。`records_indexed` 取自提交后该 generation 的实际行数，不得取暂存 vec 长度。`get_scan_status`。boot 钩子 `recover_scans(&Connection)`——boot 调用处（lib.rs）必须 log-and-continue，**不得** `.expect()`/panic（恢复失败下次 boot 重试）。
- `src-tauri/src/application/mod.rs` -- `pub mod scan;` + re-export。
- `src-tauri/src/ipc/envelope.rs` -- `ErrorEnvelope::scan_failed()` 构造器。
- `src-tauri/src/ipc/mod.rs` -- `scan_source`/`get_scan_status` 命令（lock conn→registry+scan store→application::scan）；`map_scan_error`；wrap seam + 单测。
- `src-tauri/src/lib.rs` -- invoke_handler 注册 2 命令；boot 在 migrations 后调 `application::scan::recover_scans(&conn)`（AD-16）。
- `src-tauri/tests/scan_pipeline.rs` -- 新建集成测试（tempdir 假 memories root）：I/O 矩阵全行 + fencing/CAS + stale 回收 + dirty_after_validation + SM-2 零写入 + 边界枚举。
- `src/ipc/scan.ts` -- 新建：`ScanOutcome`/`ScanStatus` 镜像 + `scanSource`/`getScanStatus` 客户端（api_version 强校验 + 形状守卫）。
- `src/ipc/errors.ts` -- stable codes 增 `scan_failed`。
- `src/features/sources/Sources.tsx` -- confirmed Source 卡片加 Scan 按钮 + 最近扫描状态标签（state + generation + 计数），aria-live 播报，键盘可达。

## Tasks & Acceptance

**Execution:**
- `src-tauri/src/index/migrations.rs` -- 追加 `Migration{id:3,name:"v2_scan_generations"}`：`scan_runs(id INTEGER PRIMARY KEY AUTOINCREMENT, source_id INTEGER NOT NULL REFERENCES source_registry(id), generation TEXT NOT NULL, state TEXT NOT NULL, fencing_token INTEGER NOT NULL, intent TEXT NOT NULL, manifest_revision TEXT NOT NULL, error_code TEXT, finished_at INTEGER)`（UNIQUE(source_id,fencing_token)）与 `memory_records(record_id TEXT NOT NULL, source_id INTEGER NOT NULL REFERENCES source_registry(id), generation TEXT NOT NULL, provider TEXT NOT NULL, unit_kind TEXT NOT NULL, native_unit_id TEXT NOT NULL, native_locator TEXT NOT NULL, content_hash TEXT NOT NULL, parser_version TEXT NOT NULL, PRIMARY KEY (record_id, generation))`（INDEX(source_id,generation)）——**record_id 不能单独作主键**（同一 record_id 会跨 generation 出现，单字段 PK + 任何 REPLACE 语义都会让 staging 覆盖 active 行，NFR-9 直接破产）；更新 schema_version 断言到 3 -- AD-5/AD-16 持久化基座。
- `src-tauri/src/domain/scan.rs` -- 状态枚举 + `Generation`/`ScanOutcome`/`ScanStatus`/`ScanError` 类型 + `fnv1a_hex` + `build_record_id`（netstring 拼接后 FNV-1a，`rec_` 前缀）-- 纯函数可测，无新依赖。
- `src-tauri/src/domain/ports/provider_adapter.rs` + `src-tauri/src/adapters/codex.rs` -- trait 增 `enumerate_file_units`（错误用具体 `EnumerateError`，禁 `Result<_,()>`）；Codex 实现矩阵边界枚举（不读正文、不含排除文件、拒绝 root 外/symlink 逃逸），`mtime` 取亚秒精度；枚举结果按 `relative_path` 排序后**去重**（in-root symlink 别名 canonicalize 后可能得到相同相对/真实路径，重复单元只保留一个）-- AD-3/AD-11 能力声明 + 计数诚实。
- `src-tauri/src/index/scan_store.rs` -- 上述全部持久化原语；commit_cas 必须单事务（CAS UPDATE 对 `state='committing'` **且** `fencing_token = 该 source 的 MAX(fencing_token)` → 0 行即回滚返回 false；成功才写 active_generation + 清旧 generation + succeeded）——CAS 必须能拒绝"已不是最新 owner"的持有者，只在持有者自己行上比较 token 等于没有 fence -- AD-28/AD-32 落地点。
- `src-tauri/src/application/scan.rs` -- 编排 + `recover_scans`；manifest 结构 `Vec<(relative_path,size,mtime_secs)>` 排序后 FNV-1a 得 `manifest_revision`；扫描中逐文件 `fs::read` 仅算 hash（不留正文）；commit 前重枚举比对 manifest -- AD-34 snapshot-at-validation。
- `src-tauri/src/ipc/{envelope,mod}.rs` + `src-tauri/src/lib.rs` -- 命令 + 注册 + boot 回收；连接打开处（boot 与测试 fixture）统一 `PRAGMA foreign_keys = ON`（两张新表都声明了 REFERENCES，必须实际强制）；boot 调 `recover_scans` 只 log 不 panic -- AD-1/AD-17 薄壳 + AD-16。
- `src-tauri/tests/scan_pipeline.rs` -- I/O 矩阵全行集成测试（含：手工构造 stale run 验证回收；**经 `application::scan_source` 公开 API 真实驱动** manifest 漂移（`ScanError::DirtyAfterValidation` + `error_code='dirty_after_validation'` + 旧 generation 保留），不允许只在 store 层伪造失败代替；第二 owner 先 begin_run 后第一 owner commit 必败的 fencing 测试；暂存后失败→上一 active 记录逐条完好（generation 隔离回归测试）；symlink 别名去重后 `records_indexed` 与实际行数一致；FK 强制下孤儿 insert 失败；SM-2 快照对比）-- 矩阵全覆盖。既有 `source_registry.rs` 中名不符实的 schema_version 测试名/注释一并修正（名字仍写 version_2 但断言 3）。
- `src/ipc/scan.ts` + `src/ipc/errors.ts` -- TS 镜像 + 新错误码 -- A-6。
- `src/features/sources/Sources.tsx` -- Scan 按钮 + 状态标签（`getScanStatus` 在 list 刷新后拉取；Scan 成功后刷新）-- AC 可见面 + NFR-13。

**Acceptance Criteria:**
- Given 一个 Confirmed Codex Source（root 含记忆文件），when Carver 点 Scan（或调 `scan_source`），then `scan_runs` 依次持久化 queued→running→staging→committing→succeeded，staging generation 只在 commit 事务 CAS 成功后切换为 active（`tessera_meta.active_generation`），返回 `ScanOutcome{outcome:'succeeded', records_indexed}`；`memory_records` 只含 file-level 记录（`unit_kind='file'`、`parser_version='file-level/v1'`、稳定 `rec_` id）。
- Given 已有 active generation 的 Source，when 扫描中途失败（文件读失败），then run 置 `failed`、staging generation 不激活、上一 active generation 及其全部记录保持可见（不出现半套索引，NFR-9）。
- Given 扫描期间源文件集合/size/mtime 变化，when commit 前最终校验执行，then generation 标记 `dirty_after_validation` 且永不激活，run 置 `failed`，active generation 不变（AD-34/AD-36）。
- Given 进程在 run 处于 `queued/running/staging/committing` 时退出，when 下次启动，then boot 回收置其为 `failed`、清理未激活 staging 记录、保留上一 active generation（AD-16）。
- Given 任意扫描路径（成功/失败/dirty），when 对比扫描前后，then Source 文件集合/内容/size/mtime 逐文件不变（SM-2 零写入测试）。
- Given `scan_source` 传未知 source_id，then 返回 `code=source_not_found`；given rejected/disabled Source，then 返回 `scan_failed` 且不写 scan 行；given 确认后 root 已删除，then 返回 `confirm_failed`。
- Given root 内含 `sessions/*.jsonl`、`CLAUDE.md` 等排除文件与 `rollout_summaries/*.md`，when 扫描成功，then 只有 Supported Artifact Matrix 内文件被索引。
- Given 任一扫描命令响应，when TS client 读取，then 版本化 envelope 往返成功且形状守卫通过；Scan 按钮键盘可达、结果经 aria-live 播报。
- Given `cargo test`，when 运行，then `scan_pipeline` 集成测试（I/O 矩阵全行）通过，且 1.1–1.3 既有测试不回归（schema_version 断言更新到 3）。

## Spec Change Log

### 2026-07-22 — bad_spec 修复（评审 pass 1，回环 1）

触发发现（6 项 bad_spec，均源于 spec 自身缺陷）：

1. `[high]` `memory_records(record_id TEXT PRIMARY KEY)` —— record_id 跨 generation 稳定（AD-15 locator-based），单字段 PK 使任何 REPLACE 语义的暂存**改写 active generation 的行**；暂存后失败/崩溃 → 上一成功版本记录永久丢失（NFR-9 直接破产，edge-case-hunter F1 + adversarial #5 同根因实证）。**修订：** schema 改 `PRIMARY KEY (record_id, generation)` + plain INSERT（migration 任务、scan_store Code Map、Design Notes"generation 隔离是物理的"）。**避免的已知坏状态：** staging 污染 active、UI 报"previous index unchanged"而旧记录已消失。
2. `[high]` commit CAS 只与持有者自己 begin_run 写下的 token 比较——自己永远匹配自己，fencing 无法拒绝并发第二 owner（adversarial #2）。**修订：** CAS WHERE 改为对每 Source `MAX(fencing_token)` 比较；新增"第二 owner 先 begin_run，第一 owner commit 必败"测试要求（Code Map、Execution、Design Notes"commit CAS 的精确语义"）。**避免的已知坏状态：** 双 owner 双 commit，输家 `DELETE` 清掉赢家刚激活的记录。
3. `[medium]` `error_code` 只在 dirty 一种失败写入，其余失败 NULL——spec 写"记失败类别"却无词汇表（adversarial #3）。**修订：** Design Notes 定义稳定词汇（`dirty_after_validation`/`read_failed`/`enumeration_failed`/`stale_recovered`/`internal`）+ `ScanError::error_code()` 映射；`fail_run` 签名必填 code；回收写 `stale_recovered`；持久化 state 解析失败必须报错禁止 `unwrap_or(Failed)`（adversarial #6 并入）。**避免的已知坏状态：** 最常见失败无持久诊断，1.8 UX 无法区分失败类别。
4. `[medium]` 编排顺序"枚举→begin_run"使首次枚举失败不留 run 行，与"失败即 fail_run"自相矛盾（adversarial #8）。**修订：** begin_run 先于首次枚举（占位 revision 后 UPDATE）（Code Map、Design Notes）。**避免的已知坏状态：** root 扫描中途不可读不留痕，`get_scan_status` 持续误报上次成功。
5. `[medium]` manifest `mtime_secs` 整秒截断——同秒同尺寸改写穿过最终校验（adversarial #7）。**修订：** `FileUnit.mtime` 亚秒精度（Code Map、Execution、Design Notes"manifest 时间精度"）。**避免的已知坏状态：** autosave 场景 stale hash 提交为 active 真相。
6. `[medium]` v2 表声明 REFERENCES 但全 crate 无 `PRAGMA foreign_keys = ON`，且 `memory_records.source_id` 无 FK（adversarial #9）。**修订：** 两张表都声明 FK + 连接打开处统一开 pragma（boot + fixture）+ 孤儿 insert 失败测试（migration 任务、ipc/lib 任务、tests 任务、Design Notes"FK 必须实际强制"）。**避免的已知坏状态：** 未来 Source 移除（AD-29）留下永远游荡的 generation 行。

**KEEP（重新派生必须保留）：** ① 状态机/分层/编排骨架与全部 18 个集成测试的行为断言方向（I/O 矩阵全行覆盖，含 `mid_scan_file_read_failure_preserves_previous_generation` 的 chmod-0o000 真实驱动法）；② `commit_cas` 单事务"CAS→active marker→清旧 gen"结构与 `INSERT OR REPLACE`→plain INSERT 之外的 store API 面；③ boot 回收 GC 语义（stale→failed + 非 active generation 行删除）；④ TS 镜像/形状守卫/错误码面；⑤ FNV-1a 无依赖 hash 与 record_id 编码；⑥ 同步命令 + std Mutex 模式（不引 tokio）；⑦ `recover_scans` 文档化的 log-and-continue 契约（boot 调用处遵守它，不得 `.expect()`——来自本 pass 的 patch 发现，重派生时一并遵守）。

### Review Findings

- [x] [Review][Patch] root 失去读权限时枚举静默为空 → 空 generation 覆盖并删除 active 记录 [`server/src/adapters/codex.rs`, `server/src/application/scan.rs`, `server/tests/scan_pipeline.rs`] — 枚举前打开 root 验证可读性；有 active generation 时拒绝意外空扫描，并覆盖回归测试。
- [x] [Review][Patch] 暂存记录用读时 realpath 构建、manifest 用枚举时快照比对，两粒度不对齐 [`server/src/application/scan.rs`, `server/tests/scan_pipeline.rs`] — manifest 已绑定枚举 canonical target；读前/后和 commit 前均重校验目标与 metadata，retarget 回归测试通过。
- [x] [Review][Patch] dirty_after_validation 的 ErrorEnvelope message 未兑现"区分源已变化" [`server/src/http/envelope.rs`, `server/src/http/mod.rs`]
- [x] [Review][Patch] clippy `io_other_error` ×3，spec Verification 要求无 warning [`server/src/lib.rs`]
- [x] [Review][Defer] 读时 containment 重校验失败接到 fail_run 而非跳过该文件 [`server/src/application/scan.rs:184-190`] — deferred，需真实并发改名竞态才可触发，与 decision 2 同根因，随其一并修订
- [x] [Review][Defer] boot 回收测试仅覆盖 staging 一种 stale 状态、单 source [`server/tests/scan_pipeline.rs:532-553`] — deferred，补 queued/running/committing 回收与多 source GC 隔离测试，属测试加固不阻塞
- [x] [Review][Defer] 损坏的 active_generation meta 被静默报 0 记录 [`server/src/index/scan_store.rs:414-425`] — deferred，正常路径不可达（仅外部 DB 编辑），与同模块"损坏要响亮"原则不一致，4.x 重建索引时一并处理

### 2026-07-23 — Parallel code review

- [x] [Review][Patch] 扫描错误扩展为携带 `source_id` 与 `phase` [`server/src/http/envelope.rs`, `server/src/http/mod.rs`, `src/api/client.ts`] — 所有 error envelope 已统一携带安全上下文；前端只接受完整契约。
- [x] [Review][Patch] 为 Scan 浏览器流程引入 Playwright 端到端验证 [`playwright.config.ts`, `tests/ui/accessibility.spec.ts`] — 键盘 Confirm → Scan、HTTP 200 与 `aria-live` 成功播报均已在隔离 fixture 上验证。
- [x] [Review][Patch] `rollout_summaries` 目录在 containment 检查前被读取 [`server/src/adapters/codex.rs`] — 先 canonicalize 并确认目录仍在 root 内；root 外 symlink 回归测试通过。
- [x] [Review][Patch] 扫描未重算并核对 Source fingerprint [`server/src/application/scan.rs`, `server/tests/scan_pipeline.rs`] — 扫描前精确重算 fingerprint，替换 root 拒绝扫描且保留 active generation。
- [x] [Review][Patch] commit 成功后的计数失败会把 succeeded run 改写为 failed [`server/src/application/scan.rs`, `server/src/index/scan_store.rs`, `server/tests/scan_pipeline.rs`] — records count 移至 CAS 前；`fail_run` 仅能改写 in-flight run。
- [x] [Review][Patch] 旧的状态请求可覆盖 Scan 后的最新状态 [`src/features/sources/Sources.tsx`] — status refresh 使用单调请求代次，只接受最新结果。

## Review Triage Log

### 2026-07-22 — Review pass 1
- intent_gap: 0
- bad_spec: 6: (high 2, medium 4, low 0)
- patch: 9: (high 0, medium 3, low 6)
- defer: 3: (high 0, medium 1, low 2)
- reject: 0
- addressed_findings:
  - `[high]` `[bad_spec]` memory_records 单字段 record_id PK → staging 覆盖 active generation 行（NFR-9 破产）；spec schema 改 composite PK + plain INSERT。
  - `[high]` `[bad_spec]` commit CAS 只比对自己 token，无法拒绝并发第二 owner；spec 改为 MAX(fencing_token) 语义 + 第二 owner 测试。
  - `[medium]` `[bad_spec]` error_code 无词汇表、仅 dirty 一种写入；spec 定义稳定词汇 + ScanError::error_code() + 回收写 stale_recovered（含 unknown state 禁静默映射 failed）。
  - `[medium]` `[bad_spec]` 枚举先于 begin_run 导致首次枚举失败不留 run 行；spec 改为 begin_run 先行（占位 revision）。
  - `[medium]` `[bad_spec]` manifest mtime 整秒截断；spec 改亚秒精度。
  - `[medium]` `[bad_spec]` REFERENCES 未强制 + memory_records 无 FK；spec 要求双表 FK + PRAGMA foreign_keys=ON + 孤儿 insert 失败测试。
  - 注：bad_spec 回环触发，代码重新派生；9 项 patch（boot recover .expect→log、NotConfirmed 专用 message、最新 run 状态展示边界、Scan 按钮并发守卫、source_registry 测试名/注释、dirty 测试改公开 API 驱动、多 Source 扫描 UI 串行提示、ScanOutcome/ScanStatus DTO 字段核对、Sources.tsx 状态获取失败诚实标签）与 3 项 defer（API_VERSION 双定义、TS 层无可执行验证、boot 接线无自动化测试）在重新派生后的下一 pass 处理。

### 2026-07-23 — Review pass 2
- patch: 10/10 已实施；defer: 保留既有 3 项，不新增。
- validation: `cargo test`（135 passed）、`cargo clippy --lib --test scan_pipeline -- -D warnings`、`npm run build`、`npm run test:e2e`（Playwright 1 passed）。
- result: 所有 blocking / high / medium review patch 已关闭；Story 进入 done。

## Design Notes

- **fencing token 方案（无 rand/时间依赖）：** `fencing_token = MAX(fencing_token)+1` per `source_id`，在 `begin_run` 的 INSERT 事务内计算——SQLite 单写者 + UNIQUE(source_id,fencing_token) 保证单调且不撞。token 与 generation（`gen_<scan_run_id>`）都出自 AUTOINCREMENT，无时钟依赖（与 1.3 `src_<rowid>` 同一先例）。
- **commit CAS 的精确语义：** `UPDATE scan_runs SET state='succeeded', finished_at=? WHERE scan_id=? AND state='committing' AND fencing_token = (SELECT MAX(fencing_token) FROM scan_runs WHERE source_id=?)`。CAS 必须与**每 Source 当前最大 token** 比较——只和持有者自己 begin_run 写下的行比较等于没有 fence（自己永远匹配自己），第二 owner 先 begin_run 后，第一 owner 的 commit 必须以 0 行失败。影响 0 行说明 owner 已失效（被回收/被取代）→ 事务回滚，`tessera_meta.active_generation` 不落。影响 1 行才继续写 active marker + 删旧 generation 记录。单事务使"CAS 成功"与"切换可见"不可分（AD-32）。1.4 同步单进程下 MAX 恒等于持有者 token，但语义按并发场景钉死并有测试（1.8 async 化时直接成立）。
- **generation 隔离是物理的：** `memory_records` 主键为 `(record_id, generation)`，staging 只做 plain INSERT。record_id 跨 generation 稳定（locator-based），因此单字段 PK + `INSERT OR REPLACE` 会在暂存时**改写 active generation 的行**，失败/崩溃后旧版本记录永久丢失——这是第 1 评审 pass 实际抓到的 NFR-9 破产路径，schema 必须物理防呆。
- **active generation 存哪：** `tessera_meta` key `active_generation:<source_rowid>`（meta 表本就是为这类标量预留的，见 `ensure_meta_tables` 注释）。不立新表。
- **为何同步扫描可接受：** 1.4 扫描是显式手动触发、单 Source、file-level（只 stat + 读 bytes 算 hash）；Carver 真实数据量级小。同步命令在 std Mutex 临界区完成全部工作，串行化天然满足 AD-5"单一 owner"，且回避 1.1 deferred 的 async 持锁死锁项。async/进度 Channel/cancellation 随 1.8 手动重扫 UX 一起引入（届时扫描移出临界区 + fencing 真正生效于并发场景；1.4 先把状态机与 CAS 语义钉死并有测试）。
- **为何不存正文：** 1.4 是骨架，证明"staging→CAS→可见性"机制；`memory_records` 只存身份 + content hash。正文列/FTS 随 1.5 的 canonical 记录落地，届时同表加列（migration v3）即可，record_id 因 locator-based 而不变（AD-15）。
- **file-level unit 的身份：** `native_unit_id` = 相对路径（如 `rollout_summaries/2026-07-01.md`）；`native_locator` = `file://<absolute>`；`record_id = rec_<fnv1a(netstring(source_id|provider|native_locator|unit_kind))>`。源未变时重扫产生相同 record_id 集合（幂等可见）；变化只体现在 content_hash（AD-15）。
- **枚举边界（AD-11 精确落法）：** 三个已知文件名在 root 第一层；`rollout_summaries/` 只取直接子文件 `*.md`（不递归子目录）；其他一切（含 `sessions/`、JSONL、规则文件）不进入 manifest。symlink：文件 realpath 必须 `starts_with` canonical root，否则跳过。
- **失败即 fail_run、不留半态（CAS 失败除外）：** begin_run 之后编排任何一步出错（枚举失败、读失败、manifest 漂移）都先 `fail_run`（state→failed + finished_at + `error_code`）再返回结构化错误。**begin_run 必须先于首次枚举**（先建行、占位 revision、枚举后 UPDATE 真实 revision）——否则首次枚举失败不留任何 run 行，状态机在最需要记录的失败模式上出现黑洞。**唯一例外是 commit CAS 失败**：CAS 影响 0 行意味着该 run 已不属于当前 owner（被回收/取代），当前持有者无权再改写它——直接返回 `scan_failed`，run 保持 `committing` 由下次 boot 回收置 `failed`。进程崩溃留下的 running/staging/committing 同样由 boot 回收兜底（AD-16）。两条路径合起来保证"不存在永远悬置的 run"。
- **error_code 稳定词汇：** `scan_runs.error_code` 非空时取以下稳定值——`dirty_after_validation`（manifest 漂移，AD-36 槽位）、`read_failed`、`enumeration_failed`、`stale_recovered`（boot 回收写入）、`internal`。`ScanError` 到词汇的映射定义在 domain 层（`ScanError::error_code()`），与 IPC code 映射分离。持久化 state 字符串解析失败必须返回错误（未知 state = 损坏），禁止 `unwrap_or(Failed)` 静默伪装。
- **manifest 时间精度：** `FileUnit.mtime` 取亚秒（nanos/micros，i64）。整秒截断会让"同秒内同尺寸改写"穿过最终校验——`snapshot-at-validation` 在 autosave 场景下静默失效。
- **FK 必须实际强制：** rusqlite 默认不开外键；`PRAGMA foreign_keys = ON` 写在连接打开处（boot + fixture），孤儿行 insert 必须失败——否则 1.8/AD-29 的 Source 移除会留下永远游荡的 generation。
- **计数诚实：** `records_indexed` = commit 后该 generation 的实际行数（`SELECT COUNT(*) WHERE generation=?`），不是暂存 vec 长度；枚举层按相对路径去重（symlink 别名）保证两数天然一致。
- **ScanOutcome/ScanStatus 无正文无路径细节：** `ScanOutcome{source_id, scan_id, generation, records_indexed}`（Ok 即成功，不带冗余 outcome 字段）；`ScanStatus{source_id, state, active_generation, active_records}`——错误细节只在 ErrorEnvelope 的 safe message 里（AD-13）。

## Verification

**Commands:**
- `cargo build` -- expected: 成功；migration v2 + domain::scan + scan_store + application::scan + 2 命令编译通过。
- `cargo test` -- expected: 通过；`scan_pipeline` 集成测试覆盖 I/O 矩阵全行（首扫成功/失败保旧/dirty_after_validation（**经公开 API 驱动**）/CAS 竞争与第二 owner fencing/崩溃回收/幂等重扫/空目录/未知与未确认 source/root 失效/零写入/边界枚举/symlink 去重计数一致/FK 强制），lib 单测（fnv1a/record_id 稳定性/wrap seam/map_scan_error/ScanError::error_code 词汇）通过，1.1–1.3 既有全绿（schema_version=3 断言更新）。
- `cargo clippy --lib --test scan_pipeline` -- expected: 无 warning。
- `npm run build` -- expected: `tsc -b && vite build` 成功（scan.ts + Sources.tsx 类型检查）。

**Manual checks (if no CLI):**
- `npm run tauri dev`：确认一个 Codex Source → 点 Scan → 状态标签显示 succeeded + generation + 记录数；再点一次（幂等）；重启应用后状态仍在（active generation 持久）。扫描期间删除 root 下某文件再扫 → 失败标签且原记录数不变。
- 端到端 IPC（2 新命令经 invoke_handler 注册）无自动化测试——同 1.1–1.3 既定残留，`tauri dev` 人工覆盖。

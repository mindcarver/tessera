---
title: Sprint Change Proposal — 弃用 Tauri，可视化改为本地网站
date: 2026-07-22
status: approved（2026-07-22 由 Carver 批准，实施中）
trigger: 用户指令「需要进行一个重大改变，不允许使用 Tauri，可视化直接用网站就好了」
scope: Major
---

# Sprint Change Proposal：弃用 Tauri 桌面壳，交付形态改为本地 Web 应用

## 1. Issue Summary（问题摘要）

### 1.1 触发点

- **触发 Story：** 非单个 Story 触发。这是产品所有者层面的技术栈约束变更，落在 Epic 1 执行早期（代码已推进 Story 1.1–1.4，sprint-status 全部仍为 backlog）。
- **问题类型：** 战略转向 / 技术栈约束变更（Strategic pivot）。
- **核心问题陈述：** 现行 PRD、架构 Spine（status: final）与已锁定 Phase 0 栈均以 **Tauri 2 桌面壳 + Tauri IPC** 为交付与传输基线。用户决定：**不允许使用 Tauri，可视化直接以网站形式交付**。所有以 Tauri 为前提的决策、文档与代码必须调整。

### 1.2 关键澄清（本次变更的真实范围）

「可视化用网站」**不等于** 产品变成纯静态网页——Tessera 的核心能力（发现/读取本机 Codex 与 Claude Code 记忆文件、SQLite FTS5 索引、notify 监听）必须有一个能访问本地文件系统的进程。浏览器受沙箱限制无法直接读任意本地文件，因此唯一自洽的落地形态是：

> **本地 Web 应用 = Rust core 内嵌 localhost HTTP 服务 + 浏览器 UI。** 启动一个本地二进制，浏览器访问 `http://127.0.0.1:<port>` 使用全部功能。

这一形态恰好命中架构 Spine Deferred 段的预设出口：*「Local HTTP/GraphQL/WebSocket transport: revisit only if an independent browser client or multi-process consumer becomes a real requirement」*——浏览器客户端现在成为真实需求，该 Deferred 项被激活。

### 1.3 什么变、什么不变（最重要结论）

- **变：** 传输层（Tauri IPC → 版本化 localhost HTTP API）、应用壳（Tauri 窗口 → 系统浏览器）、打开原始位置的实现路径（tauri-plugin-opener → 服务端调 OS open）、分发形态（Phase B 再议）、localhost 安全模型（新增）。
- **不变：** 产品定位、全部 FR/NFR 意图、只读联邦边界、离线承诺（localhost 回环不产生任何出站网络）、以及 **Rust core 的几乎全部代码**——domain / application / adapters / index / state / policy 六层按六边形架构本就与传输无关，AD-1（Rust core 是唯一应用边界）不仅不受损，反而被强化。

## 2. Impact Analysis（影响分析）

### 2.1 Epic 影响

| Epic | 影响 | 说明 |
|---|---|---|
| Epic 1（Codex 闭环） | **中高** | Story 1.1（脚手架）需重写；1.7（打开原始位置）实现路径变更；1.8（扫描进度 Channel）改 SSE/轮询；1.2–1.6、1.9 的 AC 意图不变，仅传输实现从 IPC 换为 HTTP |
| Epic 2（跨 Agent 联邦） | 低 | AC 文本基本不动；继承新传输层 |
| Epic 3（浏览与可视化） | 低 | Browse/Search 共享查询契约（AD-23）与传输无关 |
| Epic 4（健康与重建） | 低 | notify watcher、reconcile、代际切换均在 core 内，与壳无关 |
| Epic 5（Tessera Project） | 无 | 纯本地状态映射 |
| Epic 顺序 | 不变 | Codex-first 风险降级切片仍然成立 |

无 Epic 失效，无需新增 Epic，无需重排。

### 2.2 Story 影响（Epic 1 详表）

| Story | 处置 | 说明 |
|---|---|---|
| 1.1 Phase 0 脚手架 | **重写** | 栈从「Tauri 2 + IPC」改为「Rust localhost HTTP 服务 + React/Vite 浏览器 UI」；`api_version` ping 契约保留但走 HTTP；新增 loopback 绑定与安全响应头验收 |
| 1.2 Codex 发现 | 保留（实现微调） | `discover_sources` 变 HTTP 端点；AC 文本不变 |
| 1.3 Source 确认 | 保留（实现微调） | 同上；fingerprint 不上链路等不变量原样成立 |
| 1.4 扫描管线 | 保留（实现微调） | 状态机/CAS/boot 回收全部在 core，**零改动**；仅命令入口换 HTTP handler |
| 1.5 解析 canonical | 保留 | 与传输无关 |
| 1.6 搜索 + Provenance | 保留 | 与传输无关；CSP 从 tauri.conf.json 移到 HTTP 响应头 |
| 1.7 打开原始位置 | **AC 实现路径变更** | 浏览器无法调 OS open；改为服务端端点 `POST /api/open`（core 校验 `record_id` 仍在 allowlisted root 内后调用 macOS `open`）——功能可达性**不降**，因为服务端进程拥有与 Tauri 壳相同的用户权限 |
| 1.8 Inventory + 手动重扫 | 实现变更 | Tauri Channel（递增 sequence）→ SSE（`text/event-stream`）或短轮询；同步命令 + std Mutex 模式在阻塞式 HTTP handler 下依然成立 |
| 1.9 性能基准门禁 | 保留 | fixture 与四项 baseline 不变；HTTP 序列化开销纳入基准即可 |

### 2.3 产物冲突清单

**PRD（`prds/prd-tessera-2026-07-20/prd.md`）：**
- §1.1 Phase A/B 措辞（Phase B「可下载的本地个人产品」——形态待 Phase B 重议）
- UJ-1 进入状态「首次启动 Tauri 桌面应用」
- §4.7 标题描述「MVP 以 Tauri 本地桌面应用提供完整体验」
- §6.1 范围内首条「Tauri 本地桌面应用」
- §9 风险表：需新增 localhost HTTP 攻击面（DNS rebinding / 跨源）及缓解行
- §10 未决问题 #1、§11 A-2（验证环境描述含 Tauri 语境）

**Architecture Spine（status: final，需走正式修订）：**
- 范式段「React UI 通过 Tauri IPC 使用端口」
- **AD-9 需反转**：现行规则明文禁止 localhost HTTP，正是本次变更要走的路线
- AD-17（版本化有界契约）保留、更名为 API 契约
- AD-20「单一 Tauri 进程」→「单一本地服务进程」
- AD-12 补 loopback-only 绑定细则
- Stack 表（删 Tauri 2.x，加 HTTP 服务器 crate）
- Structural Seed（`src-tauri/` → 新目录名）、部署图、Capability Map FR-18 行、Deferred 段（激活 local-HTTP 项；Tauri 签名/installer 项改写为 Phase B 分发形态）

**SPEC（`_bmad-output/specs/spec-tessera/SPEC.md`）：**
- CAP-11「本机桌面应用」→「本机 Web 应用」
- Constraints 技术基线行「Tauri 本地桌面壳」→「本地 Web 应用（Rust core + localhost HTTP）」
- Non-goals「MCP/CLI/HTTP 服务」存在字面冲突，须澄清为「对外/远程 HTTP 服务」；localhost UI 服务是交付机制而非对外服务面
- Open Questions #1（Tauri patch 锁定问题失效）

**UX：** 无独立 UX 契约文档；UX-DR1–DR8 全部不变（浏览器 UI 反而让 `tests/ui/accessibility.spec.ts` 的 Playwright 直跑更容易，不再需要 Tauri WebDriver）。

**其他产物：**
- `package.json`（删 @tauri-apps/api、plugin-opener、cli）
- `src-tauri/Cargo.toml`（删 tauri/tauri-build/tauri-plugin-opener，加 HTTP 服务器 crate + `open` 或等价）
- `src-tauri/tauri.conf.json`、`capabilities/`、`build.rs`、`gen/`、`icons/`（Tauri 专属，删除）
- `docs/phase-0-verification.md`（CSP 段落随 HTTP 头方案更新，构建结论需重跑）
- `_bmad-output/implementation-artifacts/deferred-work.md`（「async Mutex」deferred 项由新传输选型一并解决/改写；Tauri 相关项清理）
- `sprint-status.yaml`：无需变更（全部 backlog，无已完成 Story 需要回滚标记）

### 2.4 技术影响（现有代码）

好消息：六边形架构在此刻兑现了价值。现有代码分三类：

- **零改动（约 85%）：** `domain/`（含 ports）、`application/`（source/scan/recover）、`adapters/`（codex/claude_code）、`index/`（registry/scan_store/migrations）、`policy/`、`state/`、`ipc/envelope.rs`（Envelope/ErrorEnvelope/API_VERSION 是纯 serde 类型）、全部 core 单测与集成测试、rusqlite/notify 依赖、`rust-toolchain.toml`。
- **改造（约 10%）：** `src-tauri/src/lib.rs`（Tauri Builder → HTTP server bootstrap；`app_data_dir` 改由 `dirs` crate 或显式路径解析）；`src-tauri/src/ipc/mod.rs`（8 个 `#[tauri::command]` → HTTP handler，envelope/错误映射/seam 测试逻辑原样搬移）；`src/ipc/*.ts`（`invoke` → `fetch`，形状守卫与契约错误逻辑保留）；`src/features/sources/Sources.tsx` 仅 import 路径微调。
- **删除：** Tauri 全部专属物（tauri.conf.json、capabilities/、build.rs 的 tauri-build、gen/、icons/、@tauri-apps/* 依赖、Cargo 中 tauri 三件套）。

**新增工作（净增）：** loopback-only 绑定与安全头（CSP 从 tauri.conf.json 平移到响应头）、同源/反 DNS-rebinding 防护、扫描进度 SSE 端点、open-original 服务端端点（1.7 时）、启动后自动打开默认浏览器。

## 3. Recommended Approach（推荐路径）

### 选项评估

| 选项 | 可行性 | Effort | Risk | 结论 |
|---|---|---|---|---|
| **1. 直接调整**（改 Story 1.1 + 传输层，保留其余） | 可行。传输恰在架构边缘 | 中 | 中（localhost 安全模型是新工作） | **采用为主路径** |
| 2. 回滚已完成工作 | 部分相关：仅传输边缘层（~15% 代码）需重写；domain/application/index 层回滚无任何收益 | 高（若全量） | 高 | 不全量采用；其「定向 rework」部分并入选项 1 |
| 3. MVP 范围重审 | MVP 功能范围**不受影响**：FR-1..FR-18 全部在新形态下可达，离线承诺不变 | — | — | 不采用，无需缩减范围 |

### 推荐方案（Hybrid：选项 1 + 定向传输层 rework）

**路径：** 修订 AD-9 并激活架构 Deferred 的 local-HTTP 项 → 重写 Story 1.1 为新栈脚手架 → 搬运现有 core 代码（仅换命令入口）→ 1.7/1.8 按新传输落地 → 其余 Story 按原序列推进。

**理由：**
1. 架构是六边形模块化单体，传输替换被 AD-1/端口设计天然隔离，改动面收敛在边缘层；
2. 架构 Deferred 段早已为「独立浏览器客户端」预设了 local-HTTP 出口，本次变更是激活预设而非破坏架构；
3. 无 Epic/Story 失效，MVP 范围与里程碑不变；
4. 主要新增风险（localhost 攻击面）可用成熟模式收敛：仅绑 127.0.0.1、同源校验、CSP 响应头、无远程端点——且这些全部可测试。

**工作量估计：** Story 1.1 重做（中）+ 传输搬运（中小）+ 1.7/1.8 新实现（中）。总体约等于把 Epic 1 前四个 Story 的传输部分重做一遍，core 逻辑测试保持绿色可作为回归网。

**风险与缓解：**
- localhost HTTP 安全面（DNS rebinding、跨站请求）→ AD-9 修订版写入硬性规则（loopback-only、Origin/Host 校验、CSP），Story 1.1 AC 增加对应验收；
- 异步/锁模式：继续沿用同步 handler + std Mutex（阻塞式服务器或 axum+`spawn_blocking`），AD-5 单一 owner 语义不被并发破坏；该决策随 Story 1.1 spec 锁定；
- 端口策略：默认固定端口、冲突可配置，仅绑回环——随 Story 1.1 spec 锁定；
- Phase B 分发形态（单二进制内嵌静态资源 + 自动开浏览器）留作 Deferred，不阻塞 MVP。

## 4. Detailed Change Proposals（详细变更提案）

> 以下为各产物的具体编辑提案（old → new）。获批准后由对应角色执行。

### 4.1 Architecture Spine

**① AD-9（核心反转）**

OLD:
```
### AD-9 — [ADOPTED] MVP transport is Tauri IPC, not localhost HTTP
- **Rule:** 请求—响应使用带 `api_version` 的 typed Tauri Commands；查询统一
  `cursor + limit`；低频状态使用 Events；扫描进度使用带递增 sequence 的
  Channels，并支持 cancellation token；不开放 localhost HTTP、WebSocket
  或远程 URL 作为默认应用面。
```

NEW:
```
### AD-9 — [REVISED 2026-07-22] MVP transport is loopback-only HTTP served by the Rust core
- **Rule:** 交付形态为本地 Web 应用：Rust core 内嵌 HTTP 服务，UI 为系统
  浏览器中的 React SPA。请求—响应使用带 `api_version` 的版本化 JSON API；
  查询统一 `cursor + limit`；扫描进度使用 SSE（递增 sequence）并支持
  cancellation token。服务**必须**仅绑定 127.0.0.1、校验 Host/Origin 以
  防 DNS rebinding 与跨站调用、响应携带收紧的 CSP 头；不监听任何外部
  网络接口，不提供任何远程访问面。（原 Tauri IPC 规则作废；浏览器客户端
  成为真实需求，激活原 Deferred 的 local-HTTP 项。）
```

**② Stack 表：** 删除 `Tauri 2.x` 行；新增「HTTP 服务器（Rust crate，Story 1.1 spec 锁定：阻塞式 rouille/tiny_http 或 axum+spawn_blocking）」行；`rusqlite`、`notify`、`React 19.2.7`、`Vite 8.1.x`、toolchain 行不变。

**③ 范式段：** 「React UI 通过 Tauri IPC 使用端口」→「React UI（浏览器 SPA）通过 loopback-only HTTP API 使用端口」；依赖方向图 `Tauri Commands / Events / Channels` → `Versioned HTTP API / SSE`。

**④ AD-20：** 「Phase A 只支持 Carver 当前本机的单一 Tauri 进程」→「单一本地服务进程（Rust 二进制内嵌 HTTP 服务 + 用户默认浏览器）」；「Tessera index/config/scan state 位于 OS-managed app-data」路径解析改由 `dirs` crate 承担。

**⑤ AD-12 追加：** 「HTTP 服务仅绑回环地址是 local-only 的组成部分；任何绑定非回环地址或新增出站调用的变更视为违反本 AD。」

**⑥ Structural Seed：** `src-tauri/` → `server/`（或保持现目录名仅去 Tauri 化，由 Story 1.1 spec 定）；`ipc/` 注释改为「versioned HTTP handlers, SSE, DTO mapping」。

**⑦ Deferred 段：** 移除「Local HTTP/GraphQL/WebSocket transport」项（已激活为 AD-9）；「公开签名、公证、installer」改写为「Phase B 分发形态（单二进制内嵌静态资源 + 自动开浏览器，或安装包）」。

### 4.2 PRD

- **§1.1 Phase A：** 「先服务 Carver 的真实开发工作流」不变；补注交付形态为「本机启动的本地 Web 应用（本地服务 + 浏览器界面）」。
- **UJ-1 进入状态：** 「首次启动 Tauri 桌面应用，无账号、无需联网」→「首次启动 Tessera 本地服务并自动打开浏览器界面，无账号、无需联网」。
- **§4.7：** 「MVP 以 Tauri 本地桌面应用提供完整体验」→「MVP 以本机本地 Web 应用提供完整体验：一个本地服务进程提供浏览器界面与全部功能，不要求账号或网络连接」。
- **§6.1 范围内：** 「Tauri 本地桌面应用」→「本地 Web 应用（Rust core 本地服务 + 浏览器 UI，仅绑回环）」。
- **§9 风险表新增行：** 「localhost HTTP 攻击面（DNS rebinding / 跨站调用）→ 本地服务被恶意网页利用 | 数据暴露 | 仅绑 127.0.0.1、Host/Origin 校验、CSP 响应头、无任何远程端点」。
- **FR-12 可验证结果**补一句：「打开原始位置由本地服务在校验路径边界后调用 OS 能力完成，浏览器本身不直接访问文件系统」。

### 4.3 SPEC

- **CAP-11 intent：** 「在无账号、无联网依赖的本机桌面应用中完成…」→「在无账号、无联网依赖的本机本地 Web 应用（本地服务 + 浏览器界面）中完成…」。
- **Constraints 技术基线行：** 「Tauri 本地桌面壳、Rust 核心、React/TypeScript/Vite UI 和 SQLite FTS5」→「本地 Web 应用：Rust 核心内嵌 loopback-only HTTP 服务、React/TypeScript/Vite 浏览器 UI、SQLite FTS5」。
- **Non-goals：** 「MCP/CLI/HTTP 服务」→「MCP/CLI、对外或远程 HTTP 服务（localhost UI 服务是交付机制，不是对外服务面）」。
- **Open Questions：** 删除 Tauri patch 锁定问题；替换为「Story 1.1 重做应锁定的 HTTP 服务器选型、端口策略与 loopback 安全验收是什么？」

### 4.4 Epics

- **A-1：** 脚手架栈改为「Rust core（HTTP 服务 crate 随 1.1 spec 锁定）+ React 19.2.7 + Vite 8.1.x + rusqlite 0.40.1(bundled) + notify 8.2.x」；目录种子 `src-tauri/` 相应调整；删除 Tauri patch/CLI patch 锁定项，替换为 HTTP 服务器 patch 锁定。
- **A-6：** 「版本化 Tauri IPC，非 localhost HTTP」→「版本化 localhost HTTP API」；Channels→SSE；其余（api_version、cursor+limit、stale_snapshot、cancellation）原样保留。
- **A-14：** 「单一 Tauri 进程」→「单一本地服务进程」。
- **Story 1.1 重写要点（Then 条款）：** 启动 Rust 二进制 → 内嵌 HTTP 服务仅绑 127.0.0.1 → 浏览器访问 UI；带 `api_version` 的 ping 端点 UI→core→UI 往返；启动无任何出站请求且服务不监听非回环地址（lsof 核验）；CSP 以响应头形式满足 A-15；`cargo test` + `npm run build` 通过；FTS5/migration v0 就绪；A-15 三项 Deferred 验证结论沿用（tokenizer/sanitizer 与传输无关，CSP 段更新为响应头方案）。
- **Story 1.7：** 「调用 OS 在对应行打开/定位」→「core 端点重新校验 `record_id` 在 allowlisted root 内后，由服务端调用 OS open（macOS `open -R`/`open`）在对应行打开/定位；浏览器不直接接触文件路径能力」。
- **Story 1.8：** 「扫描进度通过带递增 sequence 的 Channel 可见、可取消」→「…通过 SSE（递增 sequence）可见、可取消」。
- **Story 1.2/1.3/1.4 及 Epic 2–5：** AC 文本不变，实现注释中的「IPC」统一读作「HTTP API」。

### 4.5 代码执行任务清单（交接给 Dev）

1. 删除：tauri/tauri-build/tauri-plugin-opener 依赖、`tauri.conf.json`、`capabilities/`、`build.rs`（tauri-build）、`gen/`、`icons/`、`@tauri-apps/*` 三个 npm 包。
2. 改造：`lib.rs`（HTTP bootstrap + `dirs` 解析 app-data）、`ipc/mod.rs`（命令→handler，envelope 与错误映射原样）、`src/ipc/*.ts`（invoke→fetch，守卫保留）。
3. 新增：loopback 绑定 + Host/Origin 校验 + CSP 响应头；启动后自动打开默认浏览器；端口冲突处理。
4. 保留并验证：`cargo test`（core 测试应全绿，仅 IPC 层测试随 handler 形态调整）、`npm run build`；更新 `docs/phase-0-verification.md`；改写 `deferred-work.md` 中 Tauri 相关项（async Mutex 项按新传输选型关闭或改写）。

## 5. Implementation Handoff（实施交接）

**变更分级：Major** —— 触及架构 Spine 的 final 决策（AD-9 反转）与 PRD 交付形态定义。

**交接对象与职责：**

| 角色 | 职责 |
|---|---|
| **Architect（Winston）** | 执行 4.1 架构修订（AD-9 反转、Stack、Deferred 激活），将 Spine status 从 final 更新为修订版 |
| **PM（John）** | 执行 4.2 PRD 与 4.3 SPEC 修订，确认 MVP 范围不变结论 |
| **Dev（Amelia）** | 按 4.4/4.5 重写 Story 1.1 spec 并执行代码迁移；用现有 core 测试做回归网 |

**建议执行顺序：** 架构修订（半天）→ PRD/SPEC 修订（半天）→ Story 1.1 重写 + 代码迁移（主要工作）→ 1.5 起按原序列恢复。

**成功标准：**
- `cargo test`、`npm run build` 全绿，现有 core 行为测试无一回归；
- 浏览器访问本地地址可完成发现→确认→扫描→查看 Inventory 的现有功能闭环；
- 运行时 lsof 核验：进程仅监听 127.0.0.1，无任何出站连接；
- 更新后的 PRD/Spine/SPEC/Epics 四份产物措辞一致，无 Tauri 残留引用。

---

> 本提案由 bmad-correct-course 工作流于 2026-07-22 生成。按工作流门禁，**未获 Carver 明确批准前不实施任何产物编辑或代码变更**。

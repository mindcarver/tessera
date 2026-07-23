# Phase 0 验证结论（A-15 Deferred）

来源：spec `_bmad-output/implementation-artifacts/spec-1-1-phase0-scaffold.md`（Story 1.1 AC）。
回应：PRD 开放问题 #3、readiness m1、ARCHITECTURE-SPINE A-15 Deferred 项。
作用：作为 Story 1.5 / 1.6 实现路径依据；若任一项需变更架构，须同步更新 `_bmad-output/planning-artifacts/architecture/architecture-tessera-2026-07-20/ARCHITECTURE-SPINE.md` 并考虑提升为新 AD。

锁定栈实测（2026-07-21，本机；2026-07-22 传输改造后更新）：
- Rust `rustc 1.97.0 (2d8144b78 2026-07-07)`，`rust-toolchain.toml` channel = `1.97.0`。
- **2026-07-22 起不再使用 Tauri**（sprint-change-proposal-2026-07-22）：交付形态为本地 Web 应用——Rust core 内嵌 `tiny_http 0.12` loopback-only HTTP 服务 + 系统浏览器 UI；`dirs 6` 解析 OS-managed app-data；`open 5` 启动后自动打开默认浏览器。
- `rusqlite 0.40.1`（`bundled`）→ SQLite 3.x，FTS5 可用。
- 前端 React `19.2.7` + Vite `8.1.0`。
- `notify 8.2`、`thiserror 2`、`serde 1`。

构建/测试实测（2026-07-22 传输改造后）：`cargo build` ✓、`cargo test` ✓（129 项通过：unit 72（含 HTTP 层 AD-9 校验/CSP/路径消毒测试）+ 集成 57（codex_discover 9、fts5_available 2、scan_pipeline 23、source_registry 16、http_api 7 wire 级））、`npm run build` ✓。

## 1. FTS5 中文 tokenizer（trigram vs unicode61）

**方法**：用 sqlite3 CLI（3.51.0，与 bundled SQLite 3.5x 行为一致）在 4 条中文记忆样例上对比默认 `unicode61` 与 `trigram`，度量子串/短查询召回。

**实测（样例含「记忆管理是本地优先的设计」「跨Agent记忆联邦」「Codex记忆与Claude记忆」「Tessera只读索引」）**：

| 查询 | unicode61 | trigram |
|------|-----------|---------|
| `记忆`（2 字，精确） | 0 命中 | 0 命中（trigram 需 ≥3 字符） |
| `记忆*`（前缀） | 1 命中 | — |
| `记`（单字） | 0 命中 | 0 命中 |

**结论**：两个内置 tokenizer 对**中文短查询（1–2 字）召回都很差**——`unicode61` 把 CJK 当整体 token，必须靠 `*` 前缀才能子串命中；`trigram` 需 ≥3 字符，对 2 字中文词完全无能为力。这正是 A-15 Deferred 要前置验证的风险：若 Story 1.6 直接用默认 tokenizer，中文短查询会大量空结果（违反 FR-9「确实无匹配」与「未索引」必须区分的可信度）。但 FTS5 本身**可用**（trigram 对 ≥3 字符 CJK 可查询——见 `server/tests/fts5_available.rs` 的 CJK round-trip 断言；unicode61 前缀可查），并非「完全不可用」。

**对 1.5/1.6 的实现路径要求**：Story 1.6 必须在真实 Codex 中文 fixture 上度量召回/空结果率/短查询延迟后再锁定 tokenizer 方案，候选方向：
- CJK 感知分词（如导入分词器或 n-gram bigram 自定义 tokenizer），或
- 接受 `unicode61` + 查询端前缀展开（`记忆*`），或
- 评估 ICU tokenizer（需确认 bundled SQLite 是否编译入 `icu`）。
基准归 Story 1.9（`tests/benchmarks/memory-index.json`，当前阈值留空）。

**Block If #4 判定**：FTS5 **可用**（非「完全不可用」），故 spec Block If #4（「FTS5 对中文短查询完全不可用 → 提升为新 AD」）**未触发**。
**是否提升为新 AD**：**暂不**（前提是下列硬门禁生效）。
**Story 1.6 硬门禁（替代 AD 提升）**：1.6 在锁定 tokenizer 前，**必须**在 Carver 真实 Codex 中文 fixture 上度量 2 字符/3 字符短查询的召回/空结果率/延迟，并满足「FR-9 三态可区分 + 短查询召回非零」后方可锁定；否则**必须**提升为新 AD 或调整栈（如引入 ICU/CJK 分词器并验证）。本「已知实现风险」结论应同步记入 ARCHITECTURE-SPINE。

## 2. Markdown / Agent Memory 不可信内容 CSP + sanitizer

**已锁定（2026-07-22 传输改造后更新）**：CSP 从 `tauri.conf.json` 平移到 HTTP 响应头，由 `server/src/http/server.rs` 的 `CONTENT_SECURITY_POLICY` 常量统一附加到每个响应（单测 `json_responses_carry_full_security_header_set` 与 wire 测试 `ping_round_trip_carries_versioned_envelope_and_security_headers` 守护）：
`default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-src 'none'`。
- 禁远程脚本（`script-src 'self'`）、禁 raw HTML 执行入口（`object-src 'none'`、`frame-src 'none'`）、`base-uri 'self'`。
- `connect-src 'self'`：UI 只允许回连本服务自身的回环源——产品没有任何远程端点，与 local-only（AD-12/AD-20/NFR-2）一致。原 Tauri CSP 中的 `ipc:` / `http://ipc.localhost` 已删除（单测 `csp_has_no_remote_or_ipc_endpoints` 守护）；spec AC「启动无出站网络」的运行时核验（lsof）已随 2026-07-22 传输改造执行：进程仅监听 `127.0.0.1:PORT`，无其他监听与出站连接。
- AD-9 回环加固（新增，随传输改造落地并测）：服务仅绑 `127.0.0.1`；每个请求校验 `Host` 必须指向所绑回环端口（防 DNS rebinding，wire 测试 `foreign_host_header_is_rejected`）；`Origin` 存在时必须为本服务回环源（防跨站调用，wire 测试 `foreign_origin_header_is_rejected` / `own_loopback_origin_is_accepted`）；全部响应携带 `X-Content-Type-Options: nosniff` 与 `Referrer-Policy: no-referrer`，API 响应 `Cache-Control: no-store`；静态路径消毒拒绝 `..` 穿越（wire 测试 `static_path_traversal_is_rejected`）。
- `style-src 'unsafe-inline'`：**仅因 Phase 0 尚不渲染任何不可信内容而可接受**——内联 CSS 本身是 UI redressing / clickjacking / 属性泄漏向量。Story 1.5 渲染 Agent Memory 前，sanitizer **必须**剥离 untrusted HTML 的 `style` 属性与 `<style>` 元素，届时应重新评估是否可收紧 `style-src`。当前 `img-src 'self' data:` 配合 `connect-src 'self'` 已限制 CSS 外泄面；Markdown sanitizer 不得依赖内联脚本。

**sanitizer 方案（1.5 起渲染 Agent Memory 时落地）**：Phase 0 不渲染任何 Agent Memory 正文。当 Story 1.5+ 渲染 Markdown 时，必须选择**默认禁止 raw HTML 直通**的渲染路径（Rust 侧 markdown→HTML 且关闭 raw HTML，或 TS 侧 DOMPurify），并以 CSP 作纵深防御；event handler、`javascript:` URL、`<script>` 一律剥离。日志与错误信封保持 omit 正文（AD-12/AD-13，`ErrorEnvelope` 已无 body/query/credential 字段，单测 `error_envelope_omits_payload_body` 守护）。

**是否提升为新 AD**：**部分已落地**——loopback 绑定与 Host/Origin/CSP 响应头已并入修订后的 AD-9（2026-07-22）；sanitizer 是 1.5 实现细节。

## 3. 外部 SQLite `mode=ro` / WAL sidecar 可行性

**能力确认**：`rusqlite`（含 `bundled`）支持 `OpenFlags::SQLITE_OPEN_READ_ONLY` 与 WAL 模式；禁用 `immutable=1` 与 `nolock=1`（按 A-15 约束）的只读打开在栈上可行。

**Phase 0 用法**：Tessera **不**消费外部 SQLite。Derived Index 是 Tessera 自有的 bundled DB（`app_data_dir/tessera-index.db`，读写、可重建，AD-2/AD-7）。Codex/Claude Code 的 Agent Memory 是 Markdown 文件，非 SQLite。外部只读 SQLite 仅对未来 remote/local Knowledge Source（`source_kind: local_knowledge | remote_knowledge`，A-19）有意义，不在 MVP Agent Memory 路径。

**是否提升为新 AD**：**暂不**。当前无消费者；能力存在即可。

## 4. Exact toolchain build check

**结论（2026-07-22 传输改造后重跑）**：锁定栈在本机构建通过——`cargo build`、`cargo test`（129/129）、`npm run build` 均绿；Rust 1.97.0 exact patch 已锁 `rust-toolchain.toml`；tiny_http/dirs/open 与前端依赖 exact patch 由 `Cargo.lock` / `package-lock.json` 持有。原 Tauri 栈（tauri/tauri-build/tauri-plugin-opener、`tauri.conf.json`、capabilities、icons、gen）已全部移除。

**运行时核验（2026-07-22）**：真实二进制经 `TESSERA_DATA_DIR=$(mktemp)` `TESSERA_PORT=14299` 启动；`lsof` 确认进程仅监听 `127.0.0.1:14299`、无其他监听与出站连接；`curl` 验证 `/api/ping` 版本化信封与安全头、错误 Host 返回 400 `forbidden_host`、错误 Origin 返回 403 `forbidden_origin`、`/` 返回 UI `index.html`。

**仍 Deferred（不阻断 Phase 0）**：Phase B 分发形态（单二进制内嵌静态资源 + 自动开浏览器，或安装包）、公开签名、公证、自动更新、跨平台支持（AD-20）。Playwright 启动冒烟留待首个 UI 测试 Story（1.2/1.8）接入 `tests/ui/accessibility.spec.ts`——浏览器 UI 下 Playwright 直跑，不再需要 Tauri WebDriver 层。

**是否提升为新 AD**：**暂不**。build check 通过即满足 Phase 0；Deferred 项维持 AD-20 现状。

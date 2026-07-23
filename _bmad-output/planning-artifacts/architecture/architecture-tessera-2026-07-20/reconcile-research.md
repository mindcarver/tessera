# Architecture Finalize Input Reconciliation — Technical Research

## Inputs

- **Architecture spine:** `/Users/carver/workspace/mindcarver/tessera/_bmad-output/planning-artifacts/architecture/architecture-tessera-2026-07-20/ARCHITECTURE-SPINE.md`
- **Technical research:** `/Users/carver/workspace/mindcarver/tessera/_bmad-output/planning-artifacts/research/technical-codex-claude-code-hermes-openclaw-memory-integration-research-2026-07-20.md`
- **PRD addendum:** `/Users/carver/workspace/mindcarver/tessera/_bmad-output/planning-artifacts/prds/prd-tessera-2026-07-20/addendum.md`
- **Reconciliation verdict:** `substantially_aligned_with_deferred_hardening`

## 已落地决策

1. **总体架构已对齐。** Spine 明确采用 local-first、hexagonal modular monolith；Rust core 是业务、文件访问和索引边界；React WebView 只能通过 Tauri IPC 调用 application services。这直接落地了研究推荐的 Tauri 2 + Rust + React/TypeScript/Vite + SQLite FTS5 组合。
2. **所有权和只读投影已落地。** AD-2、AD-4、AD-12、AD-20 固化了“Source 是事实源、Tessera 只写自己的 Derived Index/app-data、Phase A 无网络”的原则；路径 canonicalization、confirmed root、ID-only command 和 source mutation tests 均有架构位置。
3. **Provider 能力契约已落地。** AD-3、AD-18 将 `discover/enumerate/search/watch/stable_native_ids` 与 `full/search_only/existence_only/unsupported` 纳入 domain contract，防止有限搜索结果伪装成完整枚举。
4. **扫描一致性和故障恢复已落地。** AD-5、AD-8、AD-16 将单 Source owner、staging generation、原子切换、上一 active generation 保留、启动恢复、周期 reconcile 和 watcher-as-hint 固化为架构规则，覆盖研究中的主要可靠性风险。
5. **Canonical record 与来源证据已落地。** AD-6、AD-15、Consistency Conventions 保留 native identity、locator、source hash/revision、parser version、coverage 和 observed time；Tessera Project 只能做额外映射，不能覆盖源身份。
6. **数据边界已落地。** AD-10、AD-11 及 Supported Artifact Matrix 明确区分 `agent_memory` 与未来 `local_knowledge/remote_knowledge`，排除 transcript、session、人工指令和状态库中的对话内容；Codex/Claude 当前契约与 addendum 一致。
7. **安全与可测试性已落地。** AD-13、AD-14、AD-17 及 Structural Seed 已包含结构化 source-scoped error、版本化 IPC、fixture contract、零写入、parser-version、reconcile recovery 和 capability honesty 测试。
8. **产品范围和未来方向已正确降级。** Hermes/OpenClaw、Obsidian/RAGFlow/飞书、语义检索、写回、MCP/CLI、多设备同步、HTTP/GraphQL/WebSocket 均明确 Deferred；这与 PRD 当前只支持 Codex/Claude 的 Phase A 决策一致。

## 遗漏或静默弱化的约束

- 技术研究对 FTS5 的 `trigram`/中文评测、`rank`/`snippet`、索引体积和搜索基准有较具体建议，Spine 目前只锁定“SQLite 3.x + FTS5”，没有把 tokenizer/评测门槛提升为架构约束。
- 研究要求严格 CSP、禁止远程脚本，并将 Markdown 作为不可信内容安全渲染；Spine 有源内容边界和安全测试方向，但没有独立的 CSP/渲染安全 AD，容易在 UI 实现时被静默弱化。
- 研究对外部 Agent SQLite 的 `mode=ro`、禁止 `immutable=1`/`nolock=1`、WAL sidecar 和“无法证明零写入则降级”有明确策略；Spine 当前只做文件型 Codex/Claude，未把未来 SQLite Adapter 的禁用条件写成独立决策。
- 研究把 Rust/Tauri toolchain、打包和 WebView E2E 标记为实施期需验证；Spine 的 Stack 表已经写入精确版本并称为 verified cold-start seeds，但没有对应的 build/toolchain evidence 或验收门。

## 需 Deferred / 补 AD 的 Gap

| # | Gap | 类型 | 建议处理 |
|---|---|---|---|
| 1 | **Canonical identity 语义存在内部冲突。** AD-6/AD-15 说同一 native locator 的 parser version 变化触发重解析但不改变身份；Consistency Conventions 又说 `record_id` 只在“input revision 和 parser version 不变”时稳定。技术研究支持 `source_id + native locator + unit kind` 作为身份、content hash 只做变更检测。 | 需补 AD/修正文档 | 在 Architecture Finalize 中新增或修订 Identity AD：明确 parser version 不进入 `record_id`；只有 source identity/native locator/unit kind 决定身份。parser 变化产生新解析版本和 migration/reparse 记录，不自动产生新 record ID。 |
| 2 | **Tauri CSP 与不可信 Markdown 渲染没有硬性架构门。** 研究要求严格 `default-src 'self'`、无远程脚本和安全 Markdown 子集；Spine 只在研究背景和测试列表中间接提及。 | 安全 AD | 增加 Security AD，绑定 UI/renderer：bundled assets only、严格 CSP、禁止 raw HTML/script/event handler/javascript URL、结果卡片和预览按 untrusted text 渲染，并加入 WebView E2E 验证。 |
| 3 | **FTS5 tokenizer 与搜索基线尚未成为可执行决策。** 研究建议中文/中英混合内容评测 `trigram` 与 `unicode61`，并用真实匿名语料比较召回、MRR、无结果率、延迟和索引体积；Spine 只声明 FTS5。 | Deferred AD | 保持 FTS5 为 MVP 依赖，但新增 Search Baseline AD/Deferred：Phase 0 固定 tokenizer、rank/snippet、分页与评测集；未完成基准前不得引入 vector/embedding，也不得宣称中文召回已验证。 |
| 4 | **外部 SQLite/WAL 连接器的安全边界没有独立决策。** 研究明确要求 `mode=ro`，禁止 `immutable=1`、`nolock=1`，不得创建 source sidecar，无法证明时必须降级；当前 Spine 仅在文件型 MVP 范围外隐含排除。 | Deferred AD | 保持 Phase A 文件优先；为未来 Hermes/OpenClaw/Provider SQLite 增加 Deferred AD：默认 unsupported，只有通过真实 WAL/sidecar/zero-mutation contract test 后才可启用。 |
| 5 | **精确工具链版本尚未有对应验证证据。** Spine 写入 Tauri/Rust/React/Vite/rusqlite/notify 版本，但研究只确认技术方向，明确要求实施期验证 Rust/Tauri toolchain、打包、签名和 WebView E2E。 | Deferred build gate | 将 Stack 版本视为 cold-start seed，不视为已验证能力；在 Phase 0 增加 `rust-toolchain`、Tauri build、bundled FTS5、Capability smoke、安装包启动和 WebDriver smoke 证据，失败时调整版本而不是修改产品边界。 |

## Finalize 结论

- Spine 已吸收研究的主要架构决策，当前没有阻断性方向冲突。
- 立即需要修正的是 Gap 1 的 identity 内部矛盾；它会直接影响 schema、重建和迁移实现。
- Gap 2 至 Gap 5 可以保持 Deferred，但必须在对应的 Security、Search、Provider SQLite 和 Phase 0 build gate 中留下明确 owner/验收证据。
- Hermes/OpenClaw 与 Knowledge Source 暂不进入当前实现，不应为了消除这些 Deferred 而扩大 Phase A。

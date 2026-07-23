# Architecture Finalize — PRD Input Reconciliation

## 对账范围

- **PRD：** `../../prds/prd-tessera-2026-07-20/prd.md`
- **架构脊柱：** `ARCHITECTURE-SPINE.md`
- **核对结论：** 主架构方向与当前 PRD 高度一致；核心产品约束已经被 AD-1 至 AD-20、Capability Map 和 Deferred 区域落地。仍有 4 项需要补充架构决策或显式保持 Deferred，另有 1 项脊柱内部一致性问题必须在实施前修正。

## 已落地决策

| PRD 约束 / 需求 | 架构脊柱承载 | 结论 |
|---|---|---|
| 本地优先，Agent Source 是事实源，Tessera 只拥有可重建派生索引 | AD-2、AD-12、AD-20；Deployment Envelope | 已落地 |
| Rust Core 是唯一文件、Provider、索引和查询边界；UI 只能走 IPC | AD-1、依赖方向图、AD-9、AD-17 | 已落地 |
| 只读取 Confirmed Source；路径 canonicalize、allowlist、禁止任意路径/SQL/文件句柄 | AD-4、AD-11 | 已落地 |
| Codex / Claude Code Adapter 的 capability honesty（full/search_only/existence_only/unsupported） | AD-3、AD-18、Provider Adapter contract | 已落地 |
| 排除聊天、session/transcript 和人工指令文件 | AD-11 Supported Artifact Matrix | 已落地；具体上游文件清单仍需 fixture 验证 |
| Source、Health、Coverage、Scan、Generation 分离 | AD-5、AD-7、AD-8、AD-13、AD-16 | 已落地 |
| staging generation + 原子切换；失败保留上一成功索引 | AD-5、AD-16；FR-7/8/15 映射 | 已落地 |
| Native Project 身份保留，Tessera Project 只能增加映射 | AD-6、AD-15；ER 图；FR-4/5 映射 | 已落地 |
| 搜索、浏览、Provenance、来源打开和分页 IPC | AD-6、AD-7、AD-17；Capability Map FR-9..FR-17 | 基本落地 |
| 单 Source 失败隔离，其他 Source 继续查询 | AD-5、AD-13、Failure/health model | 已落地 |
| MVP 不开放 localhost HTTP、MCP/CLI、写回、向量检索、云同步 | AD-9、AD-10、Deferred | 已落地 |
| 未来 Obsidian、RAGFlow、飞书知识库与 Agent Memory 分域 | AD-10、AD-19、Deferred | 已落地为边界和延期项 |
| Phase A 单用户、本机 macOS；公开下载/签名/更新/跨平台延期 | AD-20、Deployment & Operational Envelope | 已落地 |

## 遗漏或静默弱化的约束

### 1. NFR-13 键盘可完成性没有架构承载

PRD 要求发现、搜索、筛选和来源打开支持键盘完成（NFR-13），但脊柱的 UI/IPC/Capability Map 未形成可测试的 accessibility 约束；`src/features` 只列功能模块，没有键盘焦点、语义控件、快捷键或无障碍测试边界。

**建议：** 在 UI 架构中补一个 AD 或把 NFR-13 绑定到 UI contract：所有核心路径必须有可见焦点、键盘顺序、无鼠标替代操作和自动化验收；不能等到视觉实现阶段再解释。

### 2. NFR-11 的真实数据基准尚未成为质量闸门

PRD 明确要求用 Carver 真实数据测量搜索延迟、首次扫描、内存和索引体积，且不能编造阈值。脊柱描述了分页、并发和 FTS5，但没有定义 benchmark fixture、采样报告、基准完成前的发布条件或性能回归责任。

**建议：** 增加 Deferred/AD：Phase A 先建立匿名化真实语料与 benchmark 命令，Phase B 前必须有基线报告；在基线出现前不设置固定数值，但必须阻止无测量的性能承诺。

### 3. Browse 虽有结构映射，但缺少独立的架构契约

PRD 的 FR-16/FR-17 要求不输入查询词也能按 Provider、Tessera Project、Native Project、时间和类型浏览，并区分空集合原因。脊柱 Capability Map 将其映射到 `FR-16..FR-17`，但 AD-17 只概括“search/browse IPC”，没有明确 browse query DTO、排序/分页、空状态枚举和与 search 共享 Provenance 的契约。

**建议：** 补一个 Browse Read Port/DTO AD：浏览与搜索共享 canonical read model、cursor/limit 和 Provenance；空态必须区分 `not_scanned`、`empty`、`unavailable`；不得由 UI 自行拼装 SQL 或把搜索接口临时复用成无界列表。

### 4. Scope Policy 目前只是延期，未定义跨项目查询的安全默认值

PRD 已将 personal/domain/project/task scope 列入未决问题，脊柱也将其放入 Deferred，保留 Provider-native scope。但 FR-5/FR-10 已允许 Tessera Project 联邦映射和跨 Source 查询，若实现先于 Scope 决策，默认全局搜索可能意外混合不同领域记忆。

**建议：** 保持实现延期，但补充一个明确 Deferred gate：Phase A 只能按显式 Tessera Project/Native Project 搜索；跨项目或跨领域范围必须由用户主动扩大；在 UX/真实样本评审完成前，禁止新增隐式 global scope。

### 5. Canonical `record_id` 的稳定性规则存在脊柱内部冲突

AD-6/AD-15 规定身份由 `source_id + native locator + unit kind` 稳定生成，parser version 变化只触发重解析；但「Consistency Conventions」又写成 `record_id` 只在相同 source identity、input revision **和 parser version** 下稳定。PRD 的 FR-15/SM-6 要求重建后恢复稳定身份，这两种规则会产生不同实现结果。

**建议：** 在实施前补 AD 修正：`record_id` 不包含 content hash、input revision 或 parser version；`content_hash` 表示内容版本，`parser_version` 表示解析版本，native locator 变化才产生新的 record identity。为同一 fixture 加 parser 升级测试，验证旧记录可解释迁移。

## 需要 Deferred / 补 AD 的 Gap 清单

| Gap | 类型 | 实施前处理 |
|---|---|---|
| 键盘可完成性与无障碍验收 | 补 AD/UI contract | 在 UI 开发前写入可测试交互约束 |
| 真实数据性能基准 | Deferred gate + benchmark AD | Phase A 建立匿名语料和报告；禁止无数据阈值承诺 |
| Browse 独立读契约与空状态 | 补 AD | 固定 DTO、分页、排序、空态和 Provenance 复用规则 |
| personal/domain/project/task Scope Policy | Deferred gate | 在 UX/真实样本评审前禁止隐式跨域全局搜索 |
| record identity 规则冲突 | 必须修正 AD/Consistency Convention | 统一为 locator-based identity，并加 parser migration fixture |

## Finalize 建议

架构脊柱可以作为当前 Codex/Claude Code Phase A 的实现基底，但不能把上述五项默认为“已有架构覆盖”。其中 record identity 冲突是实施阻塞项；键盘可用性、Browse contract 和性能基准应在首个 vertical slice 之前补齐；Scope Policy 可以暂时 Deferred，但必须保留安全默认值。Hermes/OpenClaw、手动任意目录、Knowledge Source、CLI/MCP 和公开分发继续遵循脊柱 Deferred，不应在本轮偷偷扩大架构范围。

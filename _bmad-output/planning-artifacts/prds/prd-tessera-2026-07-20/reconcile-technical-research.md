# Input Reconciliation — Technical Research

## Input

- **Input name:** `technical-codex-claude-code-hermes-openclaw-memory-integration-research-2026-07-20.md`
- **Input path:** `/Users/carver/workspace/mindcarver/tessera/_bmad-output/planning-artifacts/research/technical-codex-claude-code-hermes-openclaw-memory-integration-research-2026-07-20.md`
- **Compared with:**
  - `/Users/carver/workspace/mindcarver/tessera/_bmad-output/planning-artifacts/prds/prd-tessera-2026-07-20/prd.md`
  - `/Users/carver/workspace/mindcarver/tessera/_bmad-output/planning-artifacts/prds/prd-tessera-2026-07-20/addendum.md`
- **Reconciliation verdict:** `mostly_absorbed_with_explicit_scope_override`

## 已吸收内容

1. **产品定位和所有权原则已完整吸收。** PRD 将 Tessera 定义为本地、只读的跨 Agent 记忆资产浏览器，明确 Agent Source 是事实源，Derived Index 是可删除、可重建的投影；“不迁移、不接管、不重写”的结构性差异也已进入愿景、定位和竞品风险。
2. **核心用户闭环已产品化。** 技术研究中的“发现 → 用户确认 → 扫描 → 索引 → 搜索 → 来源回溯 → 重扫/重建”已转化为 UJ-1 至 UJ-3、FR-1 至 FR-16 和相应验收结果。
3. **能力诚实与来源追踪已完整吸收。** `full`、`search_only`、`existence_only`、`unsupported`，Source Health、Native Project、Tessera Project 和 Provenance 已进入术语、功能需求、结果卡片与空结果表达。
4. **数据边界已完整吸收。** 原始聊天、session transcript、`CLAUDE.md`、`AGENTS.md`、项目规则、AI 摘要、写回、云同步、向量检索和 MCP 均被排除出 MVP；Agent Memory 与未来 Knowledge Source 被保留为独立领域类型。
5. **可靠性与安全闸门已吸收。** 零源修改、单 Source 失败隔离、上一成功索引保留、generation 完整提交、路径边界、symlink 风险、不可信 Markdown、安全日志和无遥测已分布在 PRD NFR、成功指标与 addendum 技术约束中。
6. **技术栈和架构方向已正确下沉到 addendum。** Tauri 2、Rust、React/TypeScript/Vite、SQLite FTS5、模块化单体、Tauri Capability、WebView 最小权限、只读 SQLite/WAL sidecar 风险均有保留，未污染产品需求正文。
7. **验证方式和未来演进已吸收。** PRD 使用真实本机闭环、零源修改、Provenance、离线、失败隔离和可重建性作为成功指标；addendum 保留 Obsidian、RAGFlow、飞书 Knowledge Source，以及 CLI JSON → MCP stdio 的后续顺序。

## Gaps / 冲突与建议处理

| # | Gap / 冲突 | 影响 | 建议处理 |
|---|---|---|---|
| 1 | **Provider 范围发生显式变化。** 技术研究的首版边界包含 Codex、Claude Code、Hermes 内建文件和 OpenClaw workspace；当前 PRD 将 Phase A MVP 收窄为 Codex + Claude Code，并把 Hermes/OpenClaw 放到后续。 | 若未明确声明，新架构或 Epic 可能继续按四 Connector 同期交付，造成范围失控。 | **接受 PRD 作为较新的产品决策。** Finalize 时把技术研究中的 Phase 3 标注为 post-MVP 参考；首轮架构和 Epic 只承诺 Codex、Claude Code，但 Adapter contract 不能写死为两个 Provider。 |
| 2 | **Codex/Claude 的精确 Artifact Matrix 尚未锁定。** 技术研究列出 Codex `MEMORY.md`、`memory_summary.md`、`raw_memories.md`、`rollout_summaries/`，同时排除 sessions/raw transcript/internal DB；PRD 只用“Provider 自动生成的 Agent Memory”概括，无法判定 `raw_memories.md` 和 rollout summaries 是否进入 MVP。 | Parser、数据量、隐私边界和验收 fixture 会出现不同解释，尤其 `raw` 名称容易被误判为聊天记录。 | Finalize 或架构启动前增加一张 **Supported Artifact Matrix**：逐 Provider 列出 included、excluded、conditional。建议 Codex 整理后 Markdown 为基线，`raw_memories.md`/`rollout_summaries` 只有在确认其为抽取工件而非 transcript 后纳入；内部 SQLite 默认排除。Claude 仅纳入 auto-memory `MEMORY.md` 和 topic files。 |
| 3 | **“不提供手动目录”与自定义官方目录的关系未决。** 技术研究要求尊重 `CODEX_HOME`、`CLAUDE_CONFIG_DIR`、`autoMemoryDirectory`，并讨论手动路径连接；PRD A-1 暂定自动发现失败时不提供手动目录。 | 若把“无任意目录入口”误实现为“只扫默认路径”，合法的自定义 Agent 配置会被漏掉，削弱真实闭环。 | 明确区分：**官方配置驱动的自动发现必须支持；任意目录手动添加不属于 MVP。** 只有官方配置无法读取或格式未知时显示 Candidate/Health 诊断，不允许静默回退到全盘扫描。确认后可关闭 A-1。 |
| 4 | **稳定记录身份的验收表述过宽。** FR-15/SM-6 要求重建恢复“相同稳定身份”，而技术研究的稳定 ID 依赖 `source_id + native_id/native locator`；没有 native ID 时，源文件编辑或行号移动可能合理地产生新 identity。 | 实现可能为了满足不可能的跨编辑稳定性而采用正文哈希或模糊匹配，反而破坏 Provenance。 | 将验收限定为：**在 Source 内容、Source identity、Adapter/parser 版本不变时，删除并重建得到相同 record IDs。** 源内容发生结构性编辑时只要求 Provenance 可解释；有稳定 native ID 的 Provider 才承诺跨编辑 identity。 |
| 5 | **watcher 漏事件后的最终一致性保障未成为明确验收。** addendum 说明文件事件只能触发 reconcile，但技术研究还要求启动校验、周期 full reconcile 和必要时 PollWatcher；PRD FR-8 只描述“检测变化”和手动重扫。 | 仅依赖 watcher 可能长期保留错误索引，却仍显示 healthy，违反“可信空结果”和健康诊断目标。 | 不必把实现细节写进产品正文，但应在 NFR/架构验收中明确：**watcher 不是事实源；启动时和周期性执行受限 reconcile；漏事件测试后索引能自愈。** PollWatcher 作为平台降级策略进入架构，不作为用户功能。 |

## Finalize 建议

- PRD 可以继续 Finalize；技术研究的主体结论已经被吸收，没有需要推翻定位或 MVP 的阻断性遗漏。
- Finalize 前最好直接关闭 Gap 1、Gap 3、Gap 4 的文字歧义；Gap 2 形成架构/Epic 的 Connector Artifact Matrix；Gap 5 作为架构与测试硬闸门。
- 术语实现时统一 PRD 的 `healthy` 与技术研究示例中的 `ok`，避免 Source Health 枚举在 UI、Rust domain 和 SQLite schema 之间分叉。

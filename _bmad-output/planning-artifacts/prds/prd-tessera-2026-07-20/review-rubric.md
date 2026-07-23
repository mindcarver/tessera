# PRD Quality Review — Tessera

## Overall verdict

**通过，但应在 Finalize 前处理 4 个中等问题。** 这份 PRD 的 Essential Spine 完整，产品 thesis、MVP 边界、只读隐私承诺、用户旅程和可追溯的功能需求彼此一致，已经足以进入 UX、架构与 Epic 拆解。主要风险不是方向缺失，而是自动发现失败时的恢复边界仍是未经确认的假设、Phase B 晋级信号定义不足、更新与性能要求缺少进入实现前的锁定条件，以及 SM-1 对 FR 覆盖范围表述过宽。

## Decision-readiness — adequate

§1 将阶段明确拆成自用 MVP、可下载本地产品和多知识源联邦；§5–§6 同时明确了 MVP 不接入 Hermes、OpenClaw 和知识库，也不承担写回、云同步或语义问答。这些都是可以驱动资源与范围决定的真实取舍，而不是泛泛“考虑事项”。

当前唯一直接改变首次使用闭环的未决事项是 A-1：当自动发现失败时是否允许手动添加目录。PRD 已诚实暴露该假设，但在 Finalize 后继续悬置会让 UX 和 Story 拆解基于一个用户尚未确认的范围决定。其余开放问题大多有合理的下游归属或阶段边界，不阻断 Phase A。

### Findings

- **medium** 自动发现失败时的恢复范围仍由解释代替决定（§2.4 UJ-1、§4.1 FR-1、§10 问题 1、§11 A-1）— 文档把“不必支持”解释成“不提供手动添加目录”，但仍标为 `[ASSUMPTION]`；这一选择决定首次启动空状态、异常恢复和验收用例。*Fix:* Finalize 时让用户明确确认“Phase A 仅自动发现，无手动目录入口”，或把手动添加受支持 Source 根目录纳入 FR-1/FR-2。
- **low** Phase A 验证设备范围仍未决定（§10 问题 2、§11 A-3）— A-3 已假设只在 Carver 当前 Mac 验证，但问题 2 又询问是否需要第二台机器，二者形成待裁决张力。*Fix:* 选定其一；若只锁定当前 Mac，将第二台机器验证移到 Phase B 进入条件。

## Substance over theater — strong

文档没有虚构 persona、市场规模或通用 NFR。首要用户就是首个真实操作者 Carver；三个旅程分别承担首次资产可见、跨 Agent 查询和失败可信度，均能反向解释功能组。§7 的隐私、安全和可靠性约束都直接来自“原生 Source 是事实源、Tessera 只读”的产品承诺；§4 addendum 中的竞品比较也没有把“本地、多 Agent、MCP”等通用能力包装成虚假差异。

### Findings

无实质性问题。

## Strategic coherence — strong

PRD 有清晰 thesis：不迁移、不接管、不重写，通过持续只读联邦恢复用户对原生 Agent Memory 的可见性与控制权。FR-1–FR-16 都服务于“发现—确认—索引—搜索—追溯—恢复”这一条价值链；Source Health、Coverage Level 和 Provenance 不是附属功能，而是“搜索结果可信”这一差异化主张的组成部分。

§8 的主要指标验证真实闭环、零源修改、Provenance 和离线承诺，§8.3 又明确拒绝以 Connector 数量、索引条目数量和 AI 回答感作为代理指标，战略与度量基本一致。

### Findings

- **medium** Phase B 晋级信号仍是未经确认且定义不足的活动代理（§8.2 SM-7、§10 问题 5、§11 A-2）— “连续四周主动使用”没有规定使用频次或必须成功完成的真实任务，也不能单独说明公开下载价值。*Fix:* 将其改为可观察的结果指标，例如四周内完成若干次真实跨 Agent 记忆恢复并记录失败原因；或明确它只是一项候选信号，由 Phase B 评审另定门槛。

## Done-ness clarity — adequate

FR 编号连续且每条都有可验证结果；大部分验收条件可直接转成场景测试。尤其 FR-7 的零源修改、FR-9 的三类空结果、FR-11 的 Provenance 最小字段、FR-14 的失败隔离和 FR-16 的断网闭环都具有清晰的可观察后果。

性能部分没有编造数字是正确的，但 §7.4 NFR-11 与 §10 问题 4 目前只说“技术 Spike 后锁定”，尚未定义谁负责、用什么语料、在哪个下游阶段回填阈值。FR-8 的变化检测也没有给出完成窗口；这不阻断当前产品方向，但在 Story 进入实现前必须转成验收预算。

### Findings

- **medium** 更新及时性与性能预算尚无可执行的锁定条件（§4.3 FR-8、§7.4 NFR-11–NFR-12、§10 问题 4）— “检测变化并更新”没有限定自动更新何时应可见；“技术 Spike 后锁定”没有 owner、产物或回填关口。*Fix:* 在开放问题中指定 Architecture/Spike owner、基准数据集、记录指标，以及“进入实现 Story 前将阈值回填 PRD/NFR”的退出条件；FR-8 至少区分手动重扫的完成反馈与后台 reconcile 的目标窗口。
- **low** “稳定身份”没有产品层可观察定义（§4.5 FR-15）— 重建后“恢复稳定记录身份”无法仅凭当前 PRD判断相同，是正文 hash、Source 原生 ID 还是 Provenance 组合。*Fix:* 将产品可观察结果写成“同一原始位置的书签/映射/打开目标在重建后仍指向同一记录”，具体 ID 规则留给架构；若 MVP 没有依赖稳定 ID 的用户功能，则删除此验收项。

## Scope honesty — strong

§5 与 §6 对 Phase A 的范围内外描述明确，且与 §1.1 阶段规划一致。Hermes、OpenClaw、Obsidian、RAGFlow、飞书知识库均被明确放到后续阶段；原始聊天、人工指令、写回、云端、遥测、AI 问答和任意目录也都被显式排除。远程 Knowledge Source 对“完全离线”承诺的影响在 §7.1 NFR-4、§10 问题 6 和 addendum §5 中被诚实指出。

Assumption 密度对自用 MVP 合理，且 4 个 inline 假设均进入 §11 索引。A-1 和 A-2 应在 Finalize 中确认或转成带 owner/重访条件的延期项；A-3、A-4 可保留为当前阶段假设。

### Findings

无新增问题；需处理项已列在 Decision-readiness 与 Strategic coherence。

## Downstream usability — adequate

Glossary 覆盖核心领域名词，FR、UJ、SM ID 唯一且连续，三个 UJ 都以 Carver 为具名主角。PRD 与 addendum 的职责分离清楚：产品能力和验收在正文，Tauri/Rust/SQLite、数据结构建议、竞品背景和未来接口位于 addendum，适合后续 UX、架构和 Story 工作流分别提取。

主要机械性风险是 SM-1 宣称通过 UJ-1、UJ-2 验证 FR-1 至 FR-12 和 FR-16，但这两个旅程并不能覆盖 FR-7 的零源修改、FR-8 的新增/修改/删除 reconcile、FR-10 的所有组合筛选，也不能覆盖 FR-12 的失效来源错误路径。虽然各项在其他 SM 中部分出现，当前表述会误导下游测试范围。

### Findings

- **medium** SM-1 对功能覆盖作了不可成立的宽泛声明（§8.1 SM-1）— 完成 UJ-1/UJ-2 并不自动验证 FR-1–FR-12 的全部验收条件，尤其 FR-7、FR-8、FR-10 与 FR-12 的异常路径。*Fix:* 把 SM-1 只关联真正由闭环验证的 FR，或拆成“价值闭环指标”和独立的功能验收覆盖说明；继续由 SM-2–SM-6 验证可靠性与恢复要求。

## Shape fit — strong

这是单操作者、链顶型本地工具 PRD。文档采用能力规格为主、三个负载明确的用户旅程为辅，没有用大量 persona 或界面细节拉长篇幅；同时因为它将向 UX、架构和 Epics 传递，Glossary、稳定 ID 和可验证结果的严谨度是必要的，并非过度形式化。PRD 正文与 addendum 的拆分也符合“产品定义不被实现机制淹没”的形态要求。

### Findings

无实质性问题。

## Mechanical notes

- FR-1 至 FR-16、UJ-1 至 UJ-3 均连续且无重复；SM 使用 SM-1 至 SM-7 与反指标 SM-C1 至 SM-C3，命名清晰。
- 4 处 inline `[ASSUMPTION]` 均能在 §11 找到对应 A-1 至 A-4；索引条目也都能回指正文。
- 三个 UJ 均以 Carver 为具名 protagonist。
- Glossary 中 `Derived Index` 是标准术语；正文偶见 “Tessera Derived Index” 属限定用法，不构成概念漂移。
- PRD 中的相对链接可按当前目录结构解析到 `addendum.md`、Forged Idea 和技术研究报告。
- Essential Spine 完整：愿景/定位、目标用户与 JTBD、旅程、术语、FR、非目标/MVP 范围、NFR、成功与反指标、风险、未决问题、Assumptions Index 均存在。

---
title: Tessera
status: final
created: 2026-07-20
updated: 2026-07-20
---

# PRD: Tessera

## 0. 文档目的

本 PRD 定义 Tessera 第一阶段产品：一个供个人开发者在本机使用的、只读的跨 Agent 记忆资产浏览器。本文面向产品、UX、架构和后续 Epic/Story 拆解，使用统一术语描述用户旅程、功能需求、MVP 边界与验收标准；技术机制放在 [addendum.md](./addendum.md)。本 PRD 建立在 [Forged Idea](../../../forge/user-owned-agent-brain-os/forged-idea.md) 与 [技术研究报告](../../research/technical-codex-claude-code-hermes-openclaw-memory-integration-research-2026-07-20.md) 之上，不重复其中的实现论证。

## 1. 愿景与定位

开发者会在 Codex、Claude Code 等多个 Agent 之间切换，但每个 Agent 都以自己的方式保存项目记忆。一旦用户更换工具、某个平台不可用，或者只是想知道某个项目到底留下了什么，记忆就会变成散落在不同目录里的隐形资产。

Tessera 将这些原生记忆保留在原位，通过本地、只读的方式统一清点、建立派生索引、搜索并追溯来源。它不要求用户迁移到新的记忆后端，也不替 Agent 决定应该记住什么；它让用户重新获得对已有记忆资产的可见性和控制权。

Tessera 的结构性差异不是“支持多个 Agent”，而是：**不迁移、不接管、不重写各 Agent 的原生记忆，在持续只读联邦的前提下展示真实来源、覆盖范围和健康状态。**

### 1.1 当前阶段

- **Phase A — 本机自用 MVP：** 先服务 Carver 的真实开发工作流，完整支持 Codex 与 Claude Code。交付形态为本地 Web 应用：启动一个本地服务进程，在浏览器中使用全部功能（2026-07-22 起不再使用 Tauri 桌面壳，见 sprint-change-proposal-2026-07-22）。
- **Phase B — 可分发的本地个人产品：** 在 Phase A 证明价值后，补齐分发形态（单二进制/安装包）、升级和公开使用体验。
- **Phase C — 多知识源联邦：** 连接 Obsidian、RAGFlow、飞书知识库等，但 Agent Memory 与 Knowledge Source 保持独立领域类型。

## 2. 目标用户

### 2.1 首要用户

同时使用多个编码 Agent、在多个开发项目之间切换，并希望长期掌控本地记忆资产的个人开发者。MVP 首个真实用户是 Carver。

### 2.2 Jobs To Be Done

- 当我在 Codex 与 Claude Code 之间切换时，我想统一知道它们为某个项目保存了什么，以便快速恢复上下文。
- 当某个 Agent 被替换、停用或不可访问时，我想确认其已生成的记忆仍然可定位，以便保持工作连续性。
- 当我搜索一个真实项目时，我想同时看到多个 Agent 的原始记忆及出处，以便核验信息，而不是依赖无法追溯的合成答案。
- 当某个 Source 失效、格式变化或读取失败时，我想明确知道结果是否完整，以免把“没有搜到”误解为“没有记忆”。

### 2.3 非目标用户（MVP）

- 需要团队共享、权限协作或组织级知识治理的企业团队。
- 希望 Tessera 自动从聊天中提取、改写或同步记忆的用户。
- 需要云端、多设备同步或移动端使用的用户。
- 只使用一个 Agent、且不关心其记忆资产的用户。

### 2.4 关键用户旅程

- **UJ-1. Carver 首次看见分散的 Agent Memory。**
  - **背景：** Carver 已在本机使用 Codex 和 Claude Code，并积累了多个项目的 Agent Memory。
  - **进入状态：** 首次启动 Tessera 本地服务并自动打开浏览器界面，无账号、无需联网。
  - **路径：** Tessera 自动发现 Candidate Source → Carver 查看发现依据和路径 → 逐个确认要接入的 Source → Tessera 建立 Derived Index → Source Inventory 显示两个 Provider 的项目、数量、健康和最近扫描状态。
  - **价值时刻：** Carver 第一次在同一界面确认“Codex 有什么、Claude Code 有什么”。
  - **结果：** Confirmed Source 保持启用，后续启动可直接使用。
  - **边界：** 自动发现失败时，MVP 不提供手动添加目录。

- **UJ-4. Carver 浏览一个 Agent 的记忆全貌。**
  - **背景：** Carver 还没有明确查询词，只想知道某个 Agent 为某个项目保存过哪些记忆。
  - **进入状态：** Confirmed Source 已完成至少一次成功扫描。
  - **路径：** Carver 从 Source Inventory 进入 Provider 或 Tessera Project → 按时间、Native Project 和 Agent Memory 类型浏览 → 打开任意记忆卡片查看 Provenance → 定位原始文件。
  - **价值时刻：** Carver 可以直接看到记忆资产的范围、层级和最近变化，而不必先猜关键词。
  - **结果：** 浏览与搜索共享同一 Derived Index 和健康状态。

- **UJ-2. Carver 搜索一个跨 Agent 项目。**
  - **背景：** Carver 想找回某个真实开发项目的架构决定或调试经验。
  - **进入状态：** Confirmed Source 已完成至少一次成功扫描。
  - **路径：** Carver 输入查询 → 选择 Tessera Project 或 Native Project → 按 Provider/时间筛选 → 查看来自 Codex 与 Claude Code 的结果卡片 → 打开 Provenance 对应的原始文件位置。
  - **价值时刻：** Carver 找到预期记忆，并能验证它由哪个 Agent、哪个文件、哪个位置产生。
  - **结果：** 用户在不切换 Agent 的情况下恢复项目上下文，原始 Agent Memory 未被修改。

- **UJ-3. 一个 Connector 失败但搜索仍然可信。**
  - **背景：** Claude Code 目录被移动、权限发生变化或格式暂时无法解析。
  - **进入状态：** 两个 Confirmed Source 此前都有可用 Derived Index。
  - **路径：** Tessera 检测失败 → Source Inventory 将该 Source 标记为 degraded/error 并展示原因和上次成功时间 → Codex 结果继续可查询 → Claude Code 结果明确标记为旧结果或不可用 → Carver 可触发重新扫描。
  - **价值时刻：** 用户知道哪些结果仍可信、哪些可能不完整，而不是看到静默空结果。
  - **结果：** 单个 Connector 故障不阻断其他 Source，上一可用 Derived Index 不被半次失败扫描覆盖。

## 3. 术语表

- **Agent Memory** — Codex 或 Claude Code 自动生成并持久化的记忆工件；不包括原始聊天、人工指令文件或项目规则。
- **Provider** — 产生 Agent Memory 的 Agent 产品。MVP Provider 为 Codex 与 Claude Code。
- **Connector** — Tessera 用来发现、读取和解释某一 Provider Agent Memory 的只读连接器。
- **Candidate Source** — Connector 自动发现但尚未获得用户确认的潜在 Agent Memory 来源。
- **Confirmed Source** — 用户明确允许 Tessera 读取并建立 Derived Index 的 Source。
- **Source Inventory** — 展示所有 Candidate Source 和 Confirmed Source 及其位置、Provider、Coverage Level、Source Health 和扫描状态的界面。
- **Native Project** — Provider 原生使用的项目标识、目录键或 repository 作用域；Tessera 原样保留，不猜测其含义。
- **Tessera Project** — 用户在 Tessera 中建立的项目视图，可关联来自不同 Provider 的多个 Native Project，但不修改源数据。
- **Derived Index** — Tessera 根据 Confirmed Source 在本机创建的可删除、可重建索引；不是 Agent Memory 的事实源。
- **Coverage Level** — Connector 对 Source 的真实可见能力。可取 `full`、`search_only`、`existence_only`、`unsupported`。
- **Source Health** — Source 当前可用状态。可取 `unknown`、`healthy`、`degraded`、`error`。
- **Provenance** — 一条搜索结果对应的 Provider、Source、Native Project、原始文件、定位信息、更新时间和内容标识。
- **Knowledge Source** — 未来接入的 Obsidian、RAGFlow、飞书知识库等知识来源；不是 Agent Memory。

## 4. 功能需求

### 4.1 Source 发现、确认与清点

**描述：** Tessera 在本机自动发现 Codex 和 Claude Code 的 Agent Memory，只在用户确认后读取正文并建立 Derived Index。该功能实现 UJ-1。

#### FR-1：自动发现 Candidate Source

用户启动 Tessera 后，系统可以自动发现当前本机受支持的 Codex 与 Claude Code Candidate Source。

**可验证结果：**

- 发现结果标明 Provider、候选路径、发现依据和可判定的 Native Project 信息。
- 发现阶段不读取原始聊天记录。
- 自动发现没有结果时，MVP 不显示手动添加目录入口。

#### FR-2：确认或拒绝 Source

用户可以逐个确认或拒绝 Candidate Source；只有 Confirmed Source 才能进入正文扫描和索引流程。

**可验证结果：**

- 未确认或被拒绝的 Candidate Source 不进入 Derived Index。
- 用户可以停用已确认的 Source；停用不修改或删除原始 Agent Memory。
- 确认记录能在本机应用重启后保留。

#### FR-3：查看 Source Inventory

用户可以在 Source Inventory 查看每个 Source 的 Provider、路径、Native Project、Coverage Level、Source Health、最近成功扫描时间、记录数量和最近错误。

**可验证结果：**

- 数量只在 Connector 能完整枚举时展示为完整数量。
- `search_only`、`existence_only` 或 `unsupported` 不被展示成“完整同步”。
- Source Health 变化不会删除用户已确认关系。

### 4.2 Project 联邦视图

**描述：** Tessera 保留 Provider 原生项目身份，同时允许用户用 Tessera Project 建立跨 Agent 的项目视图。该功能实现 UJ-2。

#### FR-4：保留 Native Project

系统可以按 Provider 原样保留每条 Agent Memory 的 Native Project，不把无法验证的目录键自动解释成真实 repository。

**可验证结果：**

- 无法确认的项目映射明确显示为未映射，而不是被猜测归类。
- 同一 Native Project 下的 Agent Memory 可独立搜索。

#### FR-5：建立 Tessera Project 映射

用户可以创建 Tessera Project，并将 Codex 与 Claude Code 的一个或多个 Native Project 关联到同一 Tessera Project。

**可验证结果：**

- 映射仅存在于 Tessera 本地状态，不修改 Provider 目录或文件。
- 用户可以查看、调整或移除映射。
- 移除映射不会删除任何 Agent Memory 或 Derived Index 记录。

### 4.3 只读索引

**描述：** Tessera 只索引 Agent 自动生成的 Agent Memory，并将 Derived Index 作为可重建视图。该功能支持全部用户旅程。

#### FR-6：限定 Agent Memory 边界

Connector 只能把 Provider 自动生成的 Agent Memory 纳入 Derived Index。

**可验证结果：**

- 原始聊天、session transcript、完整对话消息不进入 Derived Index。
- `CLAUDE.md`、`AGENTS.md`、项目规则和其他人工指令文件不进入 MVP Derived Index。
- 每条记录标明 Provider 内的 Agent Memory 类型，不能仅凭正文内容猜测类型。

#### FR-7：以只读方式建立 Derived Index

系统可以从 Confirmed Source 建立 Derived Index，并在任何扫描和重建过程中保持原始 Agent Memory 不变。

**可验证结果：**

- 扫描前后源文件集合、内容、大小和修改时间保持不变。
- 删除 Tessera Derived Index 后可从 Confirmed Source 重新建立。
- 失败扫描不会用不完整结果替换上一成功版本。

#### FR-8：更新 Derived Index

系统可以检测 Confirmed Source 的变化并更新 Derived Index，用户也可以手动触发 Source 重新扫描。

**可验证结果：**

- 新增、修改和删除的 Agent Memory 在成功扫描后反映到查询结果。
- 扫描过程和最终状态对用户可见。
- 手动重新扫描只作用于用户指定的 Confirmed Source。

### 4.4 跨 Agent 搜索与 Provenance

**描述：** 用户可以在一个界面搜索 Codex 与 Claude Code Agent Memory，并直接验证结果来源。该功能实现 UJ-2。

#### FR-9：搜索 Confirmed Source

用户可以输入关键词，在全部或指定 Confirmed Source 的 Derived Index 中查询 Agent Memory。

**可验证结果：**

- 默认搜索所有健康且已成功索引的 Confirmed Source。
- 查询不调用外部模型或远程搜索服务。
- 空结果区分“确实无匹配”“Source 未索引”“Source 当前不可用”。

#### FR-10：筛选搜索结果

用户可以按 Provider、Confirmed Source、Tessera Project、Native Project、Agent Memory 类型和时间筛选结果。

**可验证结果：**

- 组合筛选条件时，界面显示当前生效范围。
- 清除筛选后恢复全部 Confirmed Source 范围。

#### FR-11：展示原始结果与 Provenance

每条搜索结果必须展示原始 Agent Memory 片段及完整 Provenance，而不是自动生成的总结。

**可验证结果：**

- 每条结果至少包含 Provider、Source、Native Project、原始文件或 Provider 引用、定位信息和来源更新时间。
- 结果明确显示 Coverage Level 与 Source Health。
- Tessera 不把推断标题或项目映射伪装成 Provider 原始事实。

#### FR-12：打开原始位置

用户可以从结果卡片打开或定位 Provenance 指向的原始 Agent Memory。

**可验证结果：**

- Tessera 只打开或定位，不在应用内编辑原始文件。
- 打开/定位由本地服务在校验路径边界后调用 OS 能力完成；浏览器本身不直接访问文件系统。
- 原始位置失效时展示可理解的错误和 Source Health 状态。

### 4.5 健康、失败隔离与恢复

**描述：** Tessera 必须在 Source 不完整或不可用时明确表达可信边界，并保持其他 Source 可用。该功能实现 UJ-3。

#### FR-13：展示 Source Health

系统可以把每个 Confirmed Source 标记为 `unknown`、`healthy`、`degraded` 或 `error`，并给出可理解的原因。

**可验证结果：**

- 状态至少区分路径失效、权限不足、格式不支持和扫描失败。
- 错误展示不包含 Agent Memory 正文或凭据。

#### FR-14：隔离 Connector 失败

一个 Connector 或 Confirmed Source 失败时，用户仍可搜索其他可用 Source。

**可验证结果：**

- 单个失败不会导致全局搜索不可用。
- 失败 Source 的上一成功结果若继续展示，必须标明上次成功时间和 stale 状态。

#### FR-15：重建 Derived Index

用户可以删除并完整重建 Tessera Derived Index，而不影响 Confirmed Source 和 Tessera Project 映射。

**可验证结果：**

- 重建前明确告知只会删除 Tessera 派生数据。
- 重建后可恢复相同来源记录的稳定身份和 Provenance。
- 重建失败时原始 Agent Memory 保持不变。

### 4.6 记忆浏览与可视化

**描述：** 用户可以不输入搜索词，直接按 Provider、Tessera Project、Native Project、时间和 Agent Memory 类型浏览已索引内容。该功能实现 UJ-4。

#### FR-16：浏览 Agent Memory 集合

用户可以从 Source Inventory 或 Tessera Project 进入记忆集合，查看分页列表、最近变化和按条件筛选的 Agent Memory。

**可验证结果：**

- 浏览结果与搜索结果使用同一 Provenance、Coverage Level 和 Source Health 字段。
- 空集合明确区分“尚未扫描”“没有可索引 Agent Memory”和“Source 当前不可用”。
- 浏览列表不包含原始聊天、人工指令文件或未经确认的 Source。

#### FR-17：可视化记忆结构

用户可以通过列表、分组和状态视图理解各 Provider、Tessera Project、Native Project 和 Agent Memory 类型之间的关系。

**可验证结果：**

- 用户能从 Provider 进入项目，再进入记忆条目和原始位置。
- 视图显示最近扫描、最近变化和 Source Health，而不把派生索引状态伪装成源数据状态。
- 首版不要求知识图谱、关系自动推断或 AI 生成摘要。

### 4.7 本地使用体验

**描述：** MVP 以本机本地 Web 应用提供完整体验：一个本地服务进程提供浏览器界面与全部功能，不要求账号或网络连接。

#### FR-18：本地启动与使用

用户可以在本机启动 Tessera，完成发现、确认、扫描、搜索、打开来源和重建索引的完整闭环。

**可验证结果：**

- MVP 正常使用不要求注册、登录或配置 Tessera 云服务。
- 断网状态下，文件型 Codex 与 Claude Code Source 的全部 MVP 功能仍可使用。
- 应用退出并重启后，Confirmed Source、Tessera Project 和 Derived Index 仍然可用。

## 5. 明确非目标

- Tessera 不是新的 Agent，也不替代 Codex 或 Claude Code 的记忆生成机制。
- MVP 不导入、展示或搜索原始聊天记录。
- MVP 不编辑、删除、改写或回写原始 Agent Memory。
- MVP 不自动总结、合并、去重或裁决冲突记忆。
- MVP 不要求用户把记忆迁移到统一后端。
- MVP 不提供云上传、账号体系、多设备同步、团队协作或远程遥测。
- MVP 不接入 Hermes、OpenClaw、Obsidian、RAGFlow 或飞书知识库。
- MVP 不接入 `CLAUDE.md`、`AGENTS.md`、项目规则或其他人工指令文件。
- MVP 不提供手动添加任意目录。
- MVP 不提供 AI 问答、Embedding、向量搜索或 MCP 服务端。

## 6. MVP 范围

### 6.1 范围内

- 本地 Web 应用（Rust core 本地服务 + 浏览器 UI，仅绑回环地址）。
- Codex 与 Claude Code Connector。
- 自动发现 Candidate Source 和用户确认。
- Source Inventory、Coverage Level 与 Source Health。
- Native Project 保留与 Tessera Project 映射。
- 只读 Derived Index。
- 跨 Agent 关键词搜索和筛选。
- 原始结果卡片、Provenance 和打开原始位置。
- Source 重新扫描、失败隔离和 Derived Index 重建。
- 完全离线、无遥测、无 Tessera 云端上传。

### 6.2 MVP 范围外

- **Hermes/OpenClaw Connector：** 等 Codex 与 Claude Code 闭环稳定后再加入。
- **知识库 Connector：** Obsidian、RAGFlow、飞书知识库属于 Phase C，并保持 Knowledge Source 类型。
- **公开下载体验：** 安装、自动更新、公开文档和跨平台支持属于 Phase B。
- **语义检索与问答：** 必须先证明关键词搜索不足，再独立评估。
- **写回与同步：** 会改变 Tessera 的信任和冲突模型，需要单独产品阶段。

## 7. 跨领域约束与 NFR

### 7.1 数据所有权与隐私

- **NFR-1：** Agent Memory 始终以 Confirmed Source 为事实源，Derived Index 只能作为可重建视图。
- **NFR-2：** 正常 MVP 运行不得向 Tessera 或第三方服务器上传 Agent Memory、搜索词、项目映射或诊断数据。
- **NFR-3：** 默认不记录 Agent Memory 正文、搜索词或凭据到应用日志。
- **NFR-4：** 未来远程 Knowledge Source 只能在用户显式配置和授权后由本机连接；不得静默改变 MVP 隐私承诺。

### 7.2 安全与权限

- **NFR-5：** Tessera 只能读取用户确认的 Source 范围，不能向界面暴露任意文件读取能力。
- **NFR-6：** 任意 Source 路径变化、符号链接或权限变化都必须重新通过路径边界校验。
- **NFR-7：** 展示 Agent Memory 时必须按不可信内容处理，不能执行其中的 HTML、脚本或命令。

### 7.3 可靠性与可恢复性

- **NFR-8：** 单个 Source 失败不得阻断其他 Source 的搜索与浏览。
- **NFR-9：** 扫描必须以完整成功为可见切换条件；失败时保留上一成功 Derived Index。
- **NFR-10：** Tessera 自有索引损坏或被删除时，用户可仅依赖 Confirmed Source 完整重建。

### 7.4 性能与可用性

- **NFR-11：** 搜索延迟、首次扫描时间、内存和索引体积必须使用 Carver 的真实数据建立基准；在基准完成前不编造固定阈值。
- **NFR-12：** 扫描不应阻断用户查询上一成功 Derived Index。
- **NFR-13：** 核心发现、搜索、筛选和来源打开操作必须支持键盘完成。

## 8. 成功指标

### 8.1 主要指标

- **SM-1 — 真实闭环完成：** Carver 能在真实本机环境完成 UJ-1、UJ-2 和 UJ-4，并从同一查询中找到预先确认存在于 Codex 与 Claude Code 的 Agent Memory。验证 FR-1 至 FR-12、FR-16、FR-17。
- **SM-2 — 零源修改：** 所有受支持 Source 的验收扫描前后，源文件集合、内容、大小和修改时间没有 Tessera 导致的变化。验证 FR-7、FR-8、FR-15。
- **SM-3 — Provenance 完整：** 所有可展示的搜索结果都具有可定位的 Provider、Source、Native Project、原始位置和来源状态。验证 FR-11、FR-12。
- **SM-4 — 离线成立：** 断网环境下完成 Codex/Claude Code 的发现后使用、浏览、搜索、筛选、打开来源与重建，且没有外部网络请求。验证 FR-9、FR-16、FR-18、NFR-2。

### 8.2 次要指标

- **SM-5 — 失败隔离：** 人为使一个 Confirmed Source 不可读后，另一个 Source 仍可查询，失败 Source 显示明确状态且上一成功结果不会被半次扫描覆盖。验证 FR-13、FR-14、NFR-8、NFR-9。
- **SM-6 — 可重建性：** 删除 Derived Index 后，重建可恢复稳定记录身份、Tessera Project 映射和 Provenance。验证 FR-7、FR-15、NFR-10。
- **SM-7 — 持续自用：** `[ASSUMPTION]` Carver 连续四周在真实工作中主动使用 Tessera 回答跨 Agent 记忆问题，作为进入 Phase B 的产品价值信号。

### 8.3 反指标

- **SM-C1 — 不优化 Connector 数量：** 接入更多 Provider 不能替代 Codex 与 Claude Code 闭环的正确性和可信度。
- **SM-C2 — 不优化记忆条目数量：** 索引更多文本不是目标；聊天、人工指令和无法证明来源的内容应继续排除。
- **SM-C3 — 不以 AI 回答感取代证据：** 搜索结果的价值以可追溯性判断，不以自动生成答案的流畅度判断。

## 9. 风险与缓解

| 风险 | 产品影响 | MVP 缓解 |
|---|---|---|
| Provider 格式持续变化 | Connector 突然漏读或误读 | 显示 Coverage Level、Source Health 和 Connector 版本；失败不静默 |
| Claude Code Native Project 难以反向映射 | 记忆归错项目 | 保留原始键，无法验证时不自动映射，交给 Tessera Project 显式关联 |
| “跨 Agent 记忆”被竞品覆盖 | 产品定位失焦 | 聚焦实时只读联邦、原生资产清点、健康诊断和不迁移 |
| 本地应用权限过宽 | 暴露私人文件 | 用户确认 Source、最小读取范围、任意路径能力不进入界面 |
| localhost HTTP 攻击面（DNS rebinding / 跨站调用） | 本地服务被恶意网页利用，数据暴露 | 仅绑 127.0.0.1、Host/Origin 校验、CSP 响应头、无任何远程端点 |
| 知识库愿景拖大 MVP | Codex/Claude 闭环迟迟不完成 | Phase C 独立，MVP 数据模型仅保留扩展边界而不实现 Connector |
| 用户把空结果理解为没有记忆 | 错误决策 | 空结果同时展示扫描状态、Coverage Level 和 Source Health |

## 10. 未决问题

1. Phase A 的正式验证环境是否只锁定 Carver 当前 macOS 设备，还是要求同时在另一台机器验证？
2. Tessera Project 的创建和 Native Project 关联，是首次索引后的显式步骤，还是搜索结果中按需完成？交由 UX 设计验证。
3. 关键词搜索的基准语料、延迟预算和中文短查询行为，应在技术 Spike 后锁定。
4. 进入 Phase B 的条件除连续自用外，是否还需要明确的外部试用用户数量或反馈门槛？
5. 未来连接 RAGFlow、飞书知识库后，“完全离线”需调整为“本地优先、远程连接显式授权”；对外表述应在 Phase C 前重新确认。
6. Agent Memory 的 personal/domain/project/task 作用域是否需要在 MVP 建模、默认如何隔离，交由 UX 阶段基于真实样本决定。

## 11. Assumptions Index

- **A-1（SM-7）：** 连续四周真实自用可作为进入公开下载阶段的价值信号。
- **A-2（Phase A）：** 首个验证环境是 Carver 当前本机；跨平台不是 Phase A 验收条件。
- **A-3（搜索）：** 关键词检索足以验证首版产品价值；语义检索必须由后续评测证明必要性。
- **A-4（作用域）：** MVP 先保留 Provider 原生作用域并展示，不预设 personal/domain/project/task 的跨 Provider 默认隔离规则。

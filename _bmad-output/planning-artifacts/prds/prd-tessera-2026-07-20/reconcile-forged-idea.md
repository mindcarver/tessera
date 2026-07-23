# Input Reconciliation — Forged Idea

## Input Name

- **名称：** User-owned Agent Memory Explorer
- **来源：** `_bmad-output/forge/user-owned-agent-brain-os/forged-idea.md`
- **核对目标：** `prd.md`、`addendum.md`
- **核对结论：** 核心产品原则、只读搜索闭环和大部分非目标已被吸收；存在 3 个直接范围冲突和 1 个产品体验缺口，需要在 PRD Finalize 时显式裁决，不能静默视为已覆盖。

## 已吸收内容

| 原始输入 | 当前承载位置 | 状态 |
|---|---|---|
| Agent 可替换，记忆、知识与工作连续性归用户所有 | `prd.md`「愿景与定位」「数据所有权与隐私」 | 已完整吸收 |
| 完全本地运行、默认不上云 | `prd.md` FR-16、MVP 范围、NFR-2、SM-4 | 已完整吸收 |
| 只读取 Agent 已保存的记忆，不把原始聊天当记忆 | `prd.md` FR-6、明确非目标、SM-C2 | 已完整吸收，并进一步排除人工指令文件 |
| 自动发现 → 用户确认 → 只读索引 | `prd.md` UJ-1、FR-1/2/7/8 | 已完整吸收 |
| 来源总览、跨 Agent 搜索、可追溯原始记忆卡片、不自动合成答案 | `prd.md` Source Inventory、FR-9 至 FR-12、SM-3、SM-C3 | 基本完整吸收 |
| 不修改、删除、回写原始记忆；不强迫迁移或统一后端 | `prd.md` FR-7/12/15、明确非目标、NFR-1 | 已完整吸收 |
| 不做聊天搜索、AI 汇总、冲突裁决、云托管 | `prd.md` 明确非目标、MVP 范围外 | 已完整吸收 |
| 安装后发现来源，搜索真实项目，同时看到多个 Agent 结果并定位原文件 | `prd.md` UJ-1/2、SM-1、FR-12 | 已吸收，但当前只针对 Codex 与 Claude Code |
| 审计真实位置、格式、权限和可读取能力 | `addendum.md`「当前技术方向」「输入材料」及其引用的技术研究报告 | 已由技术研究承载，不应重复放入产品需求正文 |

## Gaps / 冲突

### 1. 首版 Provider 范围直接冲突

- **原始锁定：** MVP 接入 Codex、Claude Code、Hermes、OpenClaw。
- **当前 PRD：** Phase A/MVP 只实现 Codex 与 Claude Code；Hermes、OpenClaw 明确列为范围外。
- **影响：** 这是对原始范围的实质缩减，不是措辞差异。若未经用户确认，不能把 Forged Idea 标记为完全吸收。
- **建议处理：** 在 Finalize 中记录一个明确的产品裁决：接受“Codex + Claude Code 为可交付 MVP，Hermes + OpenClaw 为紧随其后的 Connector 阶段”，或恢复四 Provider MVP。基于技术研究的纵向切片建议，优先采用前者，但必须取得用户确认并同步更新 Forged Idea 或增加 Decision Log。

### 2. 手动路径能力直接冲突

- **原始锁定：** 自动发现之外，同时支持手动路径和连接器。
- **当前 PRD：** UJ-1、FR-1、明确非目标和 A-1 均假设自动发现失败时不提供手动添加目录。
- **影响：** 自动发现规则一旦因自定义目录、环境变量或版本变化失效，用户没有恢复主闭环的入口；同时违背已锁定输入。
- **建议处理：** Finalize 时优先恢复“受控手动 Source 路径”能力：仅允许用户选择目录、经过 Provider 探测和路径边界校验后再确认，不等于开放任意文件浏览。若仍决定排除，必须将其标为经用户确认的范围变更，而不是保留 `[ASSUMPTION]`。

### 3. 记忆作用域与默认隔离策略缺失

- **原始锁定：** 只有个人核心可默认跨领域共享；领域、项目与任务记忆默认隔离。
- **当前 PRD：** 定义了 Native Project 和 Tessera Project，但没有定义 personal/domain/project/task scope，也没有默认隔离或跨域搜索规则。
- **影响：** 全局搜索可能无意混合跨领域记忆；用户无法判断哪些内容被默认纳入跨项目查询。
- **建议处理：** 在 PRD 中增加产品级 Scope Policy：默认搜索当前 Tessera Project；跨项目/跨领域必须由用户显式扩大范围；只有可验证为 personal-core 的记录可进入全局默认范围。若 Provider 无法可靠判断 scope，则显示 `unknown`，不得自动共享。

### 4. “可视化浏览”未形成独立可验证能力

- **原始锁定：** 提供来源总览、可视化浏览和跨 Agent 搜索。
- **当前 PRD：** Source Inventory 主要浏览来源状态；记忆内容主要通过关键词搜索进入，未明确用户能否不输入查询而按 Provider/Project 浏览记忆列表。
- **影响：** 用户能“搜到已知内容”，但未必能完成“先知道我有什么”的探索式任务。
- **建议处理：** 明确一个最小 Browse Journey/Requirement：从 Source Inventory 或 Tessera Project 进入按来源、项目、时间排序的记忆列表，并复用同一原始结果卡片与 Provenance；不增加 AI 分类或新的内容模型。

## 建议处理顺序

1. **必须裁决：** 四 Provider MVP 与两 Provider MVP 的范围冲突。
2. **必须裁决：** 是否恢复受控手动路径；移除当前未获确认的 A-1 假设。
3. **建议补齐：** 记忆 scope/default isolation 的产品规则。
4. **建议补齐：** 无查询的探索式浏览闭环。
5. 完成裁决后，在 PRD 的 Decision Log 或输入对账结论中记录哪些原始锁定项被保留、延期或有意替换，避免后续 Epic/Architecture 继续使用不同范围真相。

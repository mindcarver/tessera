---
stepsCompleted: [1, 2, 3, 4, 5, 6]
inputDocuments:
  - '_bmad-output/forge/user-owned-agent-brain-os/forged-idea.md'
workflowType: 'research'
lastStep: 6
research_status: 'complete'
research_type: 'technical'
research_topic: 'Codex、Claude Code、Hermes、OpenClaw 本地记忆接入与统一可视化技术可行性'
research_goals: '审计四类 Agent 的真实记忆源、格式、权限和稳定读取方式，形成本地只读连接器、统一索引、来源追踪和 Web 可视化的 MVP 技术方案'
user_name: 'Carver'
date: '2026-07-20'
web_research_enabled: true
source_verification: true
---

# Tessera：跨 Agent 本地记忆联邦层技术研究

**Date:** 2026-07-20
**Author:** Carver
**Research Type:** Technical

---

## Executive Summary

Tessera 的技术路线可行，且跨 Agent 记忆整合的需求已被现有产品行为验证：Codex 和 Claude Code 都形成了本地持久记忆结构，Hermes 同时支持内建文件与外部 Memory Provider，OpenClaw 已提供 Codex/Claude Code 记忆导入能力。四者没有统一的存储格式、能力契约或查询协议，因此 Tessera 不应再造一个记忆系统，而应成为它们之上的本地只读联邦浏览层。

推荐 MVP 为 **Tauri 2 + Rust + React/TypeScript/Vite + SQLite FTS5** 的模块化单体。各 Agent 的源文件或 Provider 始终是事实源，Tessera SQLite 只是可删除、可重建的派生索引。产品先解决自动发现、用户确认、只读解析、跨 Agent 搜索、项目筛选、来源追踪和连接器健康展示；不读取原始聊天，不写回 Agent，不上传云端，也不在没有评测证据时引入向量数据库或 AI 摘要。

真正的技术难点不是解析 Markdown，而是诚实表达不同 Provider 的能力边界、保证零源修改、抵抗格式漂移，并在单个连接器失败时保留其余来源的可用性。建议从 Codex 完整纵向切片开始，再依次接入 Claude Code、Hermes 内建记忆和 OpenClaw workspace；Hermes Mem0 等外部 Provider 采用专属 Adapter，而不是假设存在通用全量枚举接口。

**最终判断：继续推进，但以只读 Memory Explorer 为第一阶段，并设置零源修改、来源可追溯、失败隔离和覆盖诚实四个硬性验收闸门。**

## Table of Contents

1. [Research Overview](#research-overview)
2. [Technical Research Scope Confirmation](#technical-research-scope-confirmation)
3. [Technology Stack Analysis](#technology-stack-analysis)
4. [Integration Patterns Analysis](#integration-patterns-analysis)
5. [Architectural Patterns and Design](#architectural-patterns-and-design)
6. [Implementation Approaches and Technology Adoption](#implementation-approaches-and-technology-adoption)
7. [Technical Research Recommendations](#technical-research-recommendations)
8. [Final Research Synthesis](#final-research-synthesis)
9. [Conclusion](#conclusion)

## Research Overview

本研究以“Agent 可替换、记忆资产由用户持有”为前提，验证 Codex、Claude Code、Hermes、OpenClaw 的本地记忆能否在不读取原始聊天、不修改源数据的条件下统一发现、索引、检索和可视化。方法包括：官方文档与官方仓库核验、本机安装与文件/数据库结构的只读审计、候选技术栈的运行时验证，以及对连接器覆盖范围和不确定性的显式分级。

## Technical Research Scope Confirmation

**Research Topic:** Codex、Claude Code、Hermes、OpenClaw 本地记忆接入与统一可视化技术可行性
**Research Goals:** 审计四类 Agent 的真实记忆源、格式、权限和稳定读取方式，形成本地只读连接器、统一索引、来源追踪和 Web 可视化的 MVP 技术方案。

**Technical Research Scope:**

- Architecture Analysis - 本地服务、Provider Adapter、索引层与 Web 可视化架构
- Implementation Approaches - 自动发现、用户确认、只读解析、增量索引和失败隔离
- Technology Stack - 本地运行时、搜索索引、元数据存储和前端技术选择
- Integration Patterns - 文件、配置、CLI、API、插件记忆源与手动连接器
- Performance Considerations - 增量扫描、变更检测、索引规模、查询延迟和可重建性

**Research Methodology:**

- 当前官方资料与官方仓库验证
- 本机真实安装和数据源的只读审计
- 关键技术结论的多源交叉验证
- 对不确定信息显式标注置信度与验证缺口

**Scope Confirmed:** 2026-07-20

---

<!-- Content will be appended sequentially through research workflow steps -->

## Technology Stack Analysis

### 结论摘要

MVP 推荐采用 **Tauri 2 + Rust 核心 + React/TypeScript/Vite + SQLite FTS5**。产品以本地桌面应用交付：Rust 核心负责发现、解析、索引与查询，React WebView 负责可视化；原始记忆始终留在各 Agent 的目录或 Provider 中，SQLite 只保存可删除、可重建的派生索引。

这个选择把产品边界压缩为三个必需部分：

1. Provider Adapter：只读连接不同 Agent 的记忆源；
2. Canonical Index：统一元数据、原文片段、来源与健康状态；
3. Explorer UI：跨 Agent 搜索、筛选、来源回溯和连接器诊断。

不在 MVP 引入向量数据库、云服务、LLM 摘要、记忆写回或原始聊天导入。SQLite FTS5 已提供 Unicode 分词和 trigram tokenizer，可先验证中文搜索质量；语义检索只有在真实评测证明关键词检索不足后再加入。[SQLite FTS5 官方文档](https://www.sqlite.org/fts5.html)

### 推荐技术栈

| 层 | 推荐选择 | 采用原因 | MVP 边界 |
|---|---|---|---|
| 桌面壳与核心 | Tauri 2 + Rust | 使用操作系统 WebView，不捆绑浏览器运行时；前端通过受控 IPC 调用本地能力 | 生产版不开放 localhost HTTP 服务 |
| 前端 | React + TypeScript + Vite | 适合搜索、过滤、来源卡片和连接器状态页；Vite 提供官方 React/TS 模板 | 不需要 SSR、Next.js 或复杂前端状态平台 |
| 元数据与全文索引 | SQLite + FTS5 | 单文件、事务化、可备份、可重建；可同时承载连接器状态和全文索引 | SQLite 不是记忆真相源 |
| Rust SQLite 驱动 | `rusqlite` + `bundled` | 将 SQLite 随应用构建，避免不同机器系统 SQLite/FTS5 能力不一致 | 建库时记录 schema 与 parser 版本 |
| 文件变更感知 | `notify` + debouncer | 跨 macOS/Windows/Linux 监听文件变化 | watcher 只触发重扫；正确性依赖定期 reconcile |
| Agent 接入 | 文件优先、Provider 专属 Adapter | 四个平台的“记忆”并非同一种协议，不能假设存在统一导出 API | 每个连接器声明真实能力等级 |
| 分发 | Tauri installer/bundle | 可构建平台原生安装包；后续签名和更新可独立加入 | MVP 先验证 macOS 本地分发 |

Tauri 官方架构说明其核心进程与 WebView 通过消息传递协作，并使用操作系统 WebView；权限系统可限制前端可调用的命令与路径范围。这与“只读、最小权限”的连接器模型一致。[Tauri Architecture](https://v2.tauri.app/concept/architecture/) [Tauri IPC](https://v2.tauri.app/concept/inter-process-communication/) [Tauri Permissions](https://v2.tauri.app/security/permissions/)

Vite 官方提供 `react-ts` 模板；`rusqlite` 官方仓库提供 `bundled` feature；`notify` 提供跨平台文件系统事件和 debouncer。这三项覆盖 MVP 所需的 UI、稳定 SQLite 运行时和增量扫描触发。[Vite Guide](https://vite.dev/guide/) [rusqlite](https://github.com/rusqlite/rusqlite) [notify](https://github.com/notify-rs/notify)

### Programming Languages

#### Rust：本地核心与安全边界

Rust 负责以下高价值且需要统一约束的能力：

- 解析已确认路径和 Provider 配置；
- 以只读方式读取源文件或源数据库；
- 生成内容哈希、来源定位和规范化记录；
- 写入派生 SQLite 索引；
- 提供搜索、过滤、连接器健康状态和重新索引命令；
- 用 Tauri capability/permission 限制前端可访问的命令与文件范围。

将这些能力放入一个可复用 core crate，可避免 UI 直接访问任意文件系统。未来若需要让其他 Agent 查询 Tessera，可复用同一核心，通过显式启用的 loopback API 或 Unix socket 暴露能力；这不是 MVP 的默认网络面。

#### TypeScript：可视化交互层

TypeScript 只负责视图模型与交互：连接器设置、来源确认、跨 Agent 搜索、结果卡片、来源跳转和错误展示。解析规则不应散落在前端，否则 CLI、桌面 UI 和未来 Agent 接口会产生不一致。

#### Python：研究和迁移工具，不作为产品运行时

本机 Python `sqlite3` 已验证支持 URI `mode=ro` 和 FTS5，适合审计脚本、格式探测和一次性迁移；但将 Python 打包成跨平台桌面产品会增加运行时和分发复杂度。因此它作为开发工具保留，不进入 MVP 主运行链。Python 官方 `sqlite3` 支持 URI 只读模式。[Python sqlite3 文档](https://docs.python.org/3/library/sqlite3.html#how-to-work-with-sqlite-uris)

### Development Frameworks and Libraries

#### Tauri 命令/事件，而非默认 HTTP 服务

此前产品概念中的“本地服务 + Web UI”，在桌面 MVP 中具体化为 **Tauri 本地核心进程 + WebView UI**。查询使用 command IPC，索引进度和文件变化使用 event/channel；这仍是本地服务边界，但不额外暴露端口。这样减少 CORS、端口冲突和未授权局域网访问面。[Tauri IPC](https://v2.tauri.app/concept/inter-process-communication/)

如果未来明确需要浏览器独立访问或多个 Agent 共享查询，可在同一 Rust core 之上增加 Axum/Unix socket transport，而不是重写连接器与索引逻辑。

#### Provider Adapter 能力契约

四个平台必须由独立 Adapter 解析，并向上层报告能力，而不是伪装成完全相同的数据源：

| 能力等级 | 定义 | UI 表达 |
|---|---|---|
| Full enumeration | 能稳定列出所有纳入范围的记忆条目 | 可浏览、计数、全文检索、来源回溯 |
| Search only | Provider 提供查询，但没有可靠 list/export | 显示“可搜索，不保证完整枚举” |
| Existence only | 只能确认配置或数据源存在 | 显示“已发现，尚不支持读取” |
| Unsupported/error | 未配置、无权限、格式未知或解析失败 | 显示原因、证据路径和修复建议 |

统一记录建议至少包含：`provider`、`profile`、`project_id`、`source_uri`、`source_kind`、`title`、`body`、`content_hash`、`source_mtime`、`parser_version`、`indexed_at`、`coverage_level` 和 `parse_status`。对于 Markdown，保留原文及标题层级；对于 Provider API，保留原始远端 ID 与查询边界。任何规范化都不能覆盖或回写源数据。

### Database and Storage Technologies

#### 双层存储模型

```text
Agent-owned source of truth
  ├─ Markdown files
  ├─ local SQLite/state database
  └─ configured external memory provider
            │ read-only
            ▼
Tessera derivative index
  ├─ source registry and user confirmations
  ├─ canonical memory records
  ├─ FTS5 search index
  └─ scan/error/coverage state
```

源记忆是权威数据，Tessera SQLite 是缓存和搜索结构。删除 Tessera 索引不应损失源记忆；重新扫描应得到同样的 canonical records。`content_hash + source_uri + parser_version` 可作为幂等更新基础。

#### FTS5 搜索策略

第一版使用 SQLite FTS5：

- 中文/中英混合内容优先评测 `trigram` tokenizer；
- 英文、路径和标识符可同时评测 `unicode61`；
- 结果排序在 BM25 之外加入 provider/project filter 和精确短语加权；
- 为避免索引内容与 canonical table 重复，可在验证删除/更新语义后考虑 external-content FTS5 table；
- 不把向量检索列为 MVP 依赖。

SQLite 官方文档明确列出 unicode61、porter、ascii 和 trigram tokenizer，并支持 external-content/contentless 表。[SQLite FTS5](https://www.sqlite.org/fts5.html)

`sqlite-vec` 当前官方仓库仍标注 pre-v1，并提示可能发生破坏性变化，因此只能作为后续可插拔实验，不应成为 MVP 数据格式的基础。[sqlite-vec](https://github.com/asg017/sqlite-vec)

### 四个平台的真实接入形态

#### Codex

OpenAI 当前 Codex memory pipeline 分两阶段：先从 rollout 抽取 `raw_memory` 和 `rollout_summary` 写入数据库，再把选中的内容整合到文件系统 memory workspace，包括 `raw_memories.md` 和 `rollout_summaries/`；官方实现还维护 Git baseline 并进行 secret redaction。[Codex memories README](https://github.com/openai/codex/blob/main/codex-rs/core/src/memories/README.md)

本机核验到：

- Codex CLI `0.144.5`；
- `/Users/carver/.codex/memories/MEMORY.md`、`memory_summary.md`、`raw_memories.md` 与 `rollout_summaries/`；
- `/Users/carver/.codex/memories_1.sqlite` 可用 SQLite URI `mode=ro` 打开，含 jobs 和 stage1 outputs 等内部表；
- 非 Git 记忆文件共 1506 个。

MVP 应优先把整理后的 Markdown workspace 作为用户可浏览记忆；内部 SQLite 只作为版本化、可禁用的增强连接器，不能把 `sessions` 或原始 rollout/chat 当作产品搜索内容。Codex Adapter 需要支持 `CODEX_HOME`，并对文件格式变化采用容错解析。

#### Claude Code

Claude Code 官方区分 `CLAUDE.md` 指令文件和 auto memory。Auto memory 默认存放于 `~/.claude/projects/<project>/memory/`，以 `MEMORY.md` 为入口并可链接主题文件；文件是普通 Markdown，按工作树/项目隔离，路径也可通过配置改变。[Claude Code memory docs](https://code.claude.com/docs/en/memory)

本机核验到：

- Claude Code `2.1.185`；
- `/Users/carver/.claude/projects/*/memory/MEMORY.md` 及主题文件存在；
- 共发现 110 个 auto-memory 文件；
- 用户级 `/Users/carver/.claude/CLAUDE.md` 存在，但它属于指令/上下文，应与“经验记忆”分类型展示，不能混成一种记忆。

Claude Adapter 应尊重 `CLAUDE_CONFIG_DIR` 和 `autoMemoryDirectory`。项目目录名到真实仓库路径的逆映射不是可依赖的稳定公共协议，自动推断后必须让用户确认；无法确认时保留原始 project key，而不是猜测路径。

#### Hermes

Hermes 官方提供内建文件记忆（`MEMORY.md`、`USER.md`），并允许启用一个外部 memory provider。官方 `MemoryProvider` 接口以 `read_context`、`write_context`、`search` 等操作为主，并不保证存在通用的 `list_all/export`；Mem0、Honcho、Hindsight 等 Provider 必须分别判断可枚举性。[Hermes memory guide](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory/) [Hermes MemoryProvider](https://github.com/NousResearch/hermes-agent/blob/main/agent/memory_provider.py) [Hermes memory plugins](https://github.com/NousResearch/hermes-agent/tree/main/plugins/memory)

本机核验到：

- Hermes Agent `v0.18.2 (2026.7.7.2)`；
- 官方仓库安装位于 `/Users/carver/.hermes/hermes-agent`；
- `/Users/carver/.hermes/config.yaml` 已启用 memory，当前外部 Provider 为 Mem0；
- 内建记忆文件存在。

因此 Hermes MVP 应先完整读取内建文件，再为 Mem0 做独立 Adapter。若 Provider 只能 search，则 UI 必须标记 Search only，不能用查询结果数量冒充总记忆数量。任何可能保存 whole turn/raw turn 的 Provider 字段，都要在解析层过滤，以遵守“不纳入聊天记录”的边界。

#### OpenClaw

OpenClaw 的核心记忆以 workspace Markdown 为权威来源，内建搜索使用 SQLite FTS5、向量或混合检索；官方内建实现还提供 trigram CJK 搜索和按 Agent 隔离的索引数据库。[OpenClaw memory](https://docs.openclaw.ai/concepts/memory) [OpenClaw builtin memory](https://docs.openclaw.ai/concepts/memory-builtin)

一个关键的竞争验证是：OpenClaw 自 `v2026.7.1` 起已支持从 Control UI 导入 Codex 的 `MEMORY.md`/`memory_summary.md` 和 Claude project auto-memory Markdown，明确排除 raw rollout/chat，并复制到 `memory/imports/...`，不修改源文件。[OpenClaw memory import](https://docs.openclaw.ai/concepts/memory)

这证明“跨 Agent 记忆可见性”已有真实需求，同时也划清 Tessera 的差异：

- OpenClaw 当前模式是导入副本到自身 workspace；
- Tessera 应做源数据保持原位的实时只读联邦视图；
- Tessera 同时展示来源、覆盖等级、解析健康与跨 Provider 对比，而不是成为另一个 Agent 的私有记忆仓。

本机未发现 `openclaw` CLI 或 `~/.openclaw`，所以当前只能实现官方格式的 Adapter 与手动路径连接，不能声称已经完成本机真实数据验证。

### Development Tools and Platforms

本机开发环境核验结果：

| 工具 | 本机状态 | 对方案的影响 |
|---|---|---|
| Rust/Tauri | 尚需在实施阶段核验完整 toolchain | 技术选择成立，但不能据此声称本机已可构建 Tauri |
| Node.js | `v22.23.1` | 满足当前 Vite 的 Node 版本要求 |
| Python | `3.12.13` | 可用于研究脚本和格式审计 |
| SQLite CLI | `3.51.0`，FTS5 可用 | 可做 schema/查询验证 |
| Python sqlite3 | SQLite `3.53.3`，FTS5 可用 | 已验证只读 URI 打开 Codex DB |
| Node `node:sqlite` | SQLite `3.51.3`，本机仍产生 ExperimentalWarning | 不建议作为 MVP 稳定存储层 |
| `uv` | `0.6.5` | 仅用于开发辅助脚本 |
| `rg` | `15.1` | 适合文件发现与调试，不作为应用搜索引擎 |

Node 官方当前将 `node:sqlite` 标记为 release candidate，但本机 Node 22 仍显示 ExperimentalWarning；选择 Rust + bundled SQLite 可减少 Node 版本差异带来的分发风险。[Node SQLite API](https://nodejs.org/api/sqlite.html)

建议测试工具保持最小但覆盖真实风险：Rust unit/integration tests 验证解析器、路径限制和索引幂等性；前端 component tests 验证过滤与覆盖等级；fixture-based connector tests 固定四个平台的匿名化样本；端到端测试验证“发现 → 确认 → 扫描 → 搜索 → 打开来源”。

### Cloud Infrastructure and Deployment

MVP 不需要云基础设施：

- 不上传记忆、索引或遥测；
- 不依赖托管数据库、对象存储或向量服务；
- 默认不监听 TCP 端口；
- 安装包通过 Tauri 的平台 bundler 生成，正式发布再加入 macOS 签名与 notarization。

Tauri 官方支持构建各平台 installer/bundle，并说明多数平台的签名是分发所需步骤。[Tauri Distribution](https://v2.tauri.app/distribute/)

备份策略也应遵循派生索引原则：备份用户确认的 connector registry 和应用设置即可；索引数据库可随时重建。若未来提供“导出”，导出对象应是来源清单、健康报告或用户明确选择的记忆副本，不能默认复制全部 Provider 数据。

### Technology Adoption Trends and Alternatives

#### 已验证的趋势

1. **记忆正在从单一文件演化为文件 + 派生索引。** Codex 有抽取数据库与文件 workspace，Claude 保持 Markdown auto-memory，OpenClaw 以 Markdown 为真相源并建立 SQLite 搜索索引。
2. **跨 Agent 导入已出现。** OpenClaw 的 Codex/Claude memory import 直接验证了用户希望复用既有记忆，但其复制模型仍留下实时性、重复数据和来源治理空间。
3. **Provider 抽象不等于可迁移性。** Hermes 的 provider interface 能写、读上下文和搜索，却不保证完整枚举；统一 UI 必须诚实暴露能力差异。
4. **本地关键词检索仍是合理基线。** SQLite FTS5 已覆盖全文、短语、排名和 trigram；向量方案不应在没有离线评测集时先验加入。

#### 暂不采用的替代方案

| 方案 | 暂不采用原因 | 重新评估条件 |
|---|---|---|
| Electron | 捆绑 Chromium，应用体积和运行时开销高于当前需求 | Tauri WebView 兼容性无法满足关键 UI |
| Node/Fastify + browser UI | 需要管理本地端口、后台服务与 Node 分发；本机 `node:sqlite` 状态仍有版本差异 | 产品明确要求独立浏览器和多客户端并发 |
| Python/FastAPI | 原型快，但跨平台桌面打包和运行时管理更复杂 | 团队 Rust 能力成为实际交付阻塞 |
| Go/Wails | 可行的轻量候选，但当前 SQLite/解析/IPC 方案用 Rust/Tauri 更一致 | Rust 构建或维护成本经验证过高 |
| Qdrant/Chroma/Elasticsearch | 对本地单用户 MVP 属于额外服务与运维负担 | 数据规模和评测证明 SQLite 无法满足延迟/召回 |
| sqlite-vec/云 embedding | pre-v1 或引入模型、隐私、成本与可重现性问题 | 有匿名评测集证明语义检索产生显著增益 |

### 风险、验证缺口与置信度

| 结论 | 置信度 | 已验证证据 | 尚需验证 |
|---|---|---|---|
| Codex/Claude Markdown 可做只读 MVP | 高 | 官方文档 + 本机真实文件 | 多版本 fixture 和格式漂移 |
| Hermes 内建文件可枚举 | 高 | 官方文档/代码 + 本机存在 | profile、多 Provider 版本差异 |
| Hermes 外部 Provider 可统一完整枚举 | 低 | 官方接口不保证 list/export | Mem0 实例 API、分页、删除与权限语义 |
| OpenClaw Markdown/内建索引结构可接入 | 中高 | 官方文档与仓库 | 本机未安装，需真实 fixture/安装验证 |
| Tauri/Rust/SQLite 是 MVP 合适栈 | 中高 | 官方能力 + 本机 SQLite/Node 对比 | Rust/Tauri toolchain、打包、签名与 WebView E2E |
| FTS5 trigram 足够满足中文搜索 | 中 | SQLite 官方能力 | 需要用真实匿名语料测召回、误匹配和索引体积 |

**第 2 步技术判断：** 方案在本地只读 MVP 范围内可行，但“统一管理”必须定义为统一发现、统一索引、统一搜索和统一来源可视化，而不是假设所有 Provider 都能导出全部记忆。最关键的工程控制面不是新的记忆格式，而是连接器能力声明、用户确认、来源追踪、只读权限和索引可重建性。

## Integration Patterns Analysis

### 研究覆盖与总判断

本步骤核验了四类 Agent 的官方记忆入口、Tauri 2 IPC 与权限模型、SQLite 只读语义、跨平台文件监听以及未来 Agent 查询协议。结论是：MVP 不需要微服务、API Gateway、消息队列或常驻 HTTP 服务；正确模式是 **进程内 Adapter + 明确能力协商 + 用户确认的 Source Registry + 事务化派生索引**。

```text
Bundled React UI
       │ Tauri Commands / Channels / Events
       ▼
Tauri Rust Core
  ├─ Source Registry + Path Guard
  ├─ Provider Adapters
  ├─ Watcher + Reconciler
  ├─ Query Service
  └─ SQLite/FTS5 derivative index
       ▲
       │ read-only
Codex / Claude Code / Hermes / OpenClaw memory sources
```

跨平台统一的对象不是源格式，而是以下事实：来源是谁、属于哪个 profile/project、能否完整枚举、原文在哪里、当前是否健康、这条结果是否来自完整索引。任何 Adapter 无法证明的信息都必须为空或显示待确认，不能用推断补齐。

### API Design Patterns

#### MVP：Tauri Command 作为请求—响应 API

前端只通过小型、具体的 Tauri Commands 调用 Rust 核心：

```text
discover_sources()
confirm_source(candidate_id)
list_sources(filters, cursor)
search_memories(query, filters, cursor)
get_memory(record_id)
start_rescan(source_id, progress_channel)
get_source_health(source_id)
```

Tauri Commands 支持参数、返回值、错误和 async，适合发现、搜索、读取和重扫；Events 是异步、无返回值、只支持 JSON 且不具备类型安全，因此只用于低频状态通知；长时间扫描的有序进度使用 Channel。[Tauri Calling Rust](https://v2.tauri.app/develop/calling-rust/) [Tauri IPC](https://v2.tauri.app/concept/inter-process-communication/)

Command 必须只接受 `candidate_id`、`source_id`、`record_id` 等受控标识符，禁止暴露 `read_arbitrary_file(path)`、`execute_sql(sql)` 或 `scan_path(path)` 之类通用入口。WebView 不接收文件句柄、SQLite 连接或 Provider 凭据。

#### REST、GraphQL、gRPC 与 Webhook 的取舍

| 模式 | MVP 判断 | 原因 | 重新评估条件 |
|---|---|---|---|
| localhost REST | 默认不采用 | UI 与核心同一桌面进程；端口、CORS、鉴权和 DNS rebinding 没有当前收益 | 独立浏览器或多个本地客户端成为明确需求 |
| GraphQL | 不采用 | 当前查询固定、数据模型小；resolver、查询复杂度和字段授权属于额外成本 | 出现多个外部团队和大量自由组合查询 |
| gRPC/Protobuf | 不采用 | 没有跨机器微服务、高吞吐二进制 RPC 或双向流需求 | 核心被拆成多语言独立服务 |
| Webhook | 不采用 | 四类本地源没有统一推送协议；文件 watcher 已足够做失效提示 | Provider 提供经过验证、可认证的变更订阅 |
| WebSocket | 不采用 | 桌面 IPC 已覆盖低频状态与扫描进度 | 独立浏览器需要长期双向连接 |

gRPC 官方定位包括跨服务 RPC、HTTP/2、Protobuf 和流式通信；这些不是当前单进程拓扑需要解决的问题。[gRPC Introduction](https://grpc.io/docs/what-is-grpc/introduction/)

#### 未来：CLI JSON + MCP stdio

为了让其他 Agent 在未来查询 Tessera，应先让 Query Service 与 transport 解耦，然后按以下顺序增加入口：

1. `tessera search --json`，作为能执行本地命令的 Agent 的最低依赖接口；
2. MCP stdio server，向支持 MCP 的 Agent 暴露只读工具；
3. 只有出现多客户端共享 daemon 的真实需求后，再考虑 Streamable HTTP。

MCP 使用 JSON-RPC，支持 stdio 和 Streamable HTTP；本地 stdio 通常由客户端显式启动单个子进程，无需开放监听端口。[MCP Architecture](https://modelcontextprotocol.io/docs/learn/architecture) [MCP Protocol Overview](https://modelcontextprotocol.io/specification/2025-11-25/basic)

建议未来只暴露：`search_memories`、`get_memory`、`list_sources`、`get_source_health`。工具声明的 `readOnlyHint` 只能作为客户端提示，真正的只读边界仍必须由 Rust 核心强制执行。MCP 不进入本次 MVP，以免把“以后 Agent 使用记忆”与“现在先看清现有记忆”混在一起。

### Communication Protocols

#### 桌面内部通信

| 通道 | 语义 | 使用场景 |
|---|---|---|
| Tauri Command | 请求—响应、可返回错误 | 搜索、分页、读取、确认来源、启动扫描 |
| Tauri Channel | 有序流式进度 | 首次全量扫描、重建索引 |
| Tauri Event | 无权威返回值的状态提示 | `source-dirty`、`scan-state-changed` |
| Rust bounded channel | 核心内部任务协作 | 合并 scan 请求、限制并发、背压 |

Event 只表达“状态可能变化”；前端收到后必须再通过 Command 获取权威状态。文件读取、来源确认、重新索引等特权行为不能由 Event 直接触发。

#### 不采用消息队列

RabbitMQ/AMQP、Kafka、MQTT 等 broker 解决跨进程或跨机器生产者—消费者解耦。本产品 MVP 只有一个本地核心进程和一个 UI，没有引入 broker、投递保证、死信队列和运维面的必要。进程内有界 channel 足够；若以后扫描任务真正拆分到多个进程或机器，再重新评估。[RabbitMQ Tutorial](https://www.rabbitmq.com/tutorials/tutorial-one-python)

### Data Formats and Standards

#### Source Registry

Source Registry 保存用户确认过的连接，不保存原 Provider 的正文、token、Cookie 或配置文件原文：

```json
{
  "schema_version": "1.0",
  "source_id": "src_...",
  "provider_type": "codex",
  "adapter_id": "codex-markdown",
  "adapter_version": "1.0.0",
  "display_name": "Codex default profile",
  "root_uri": "file:///Users/.../.codex/memories/",
  "scope": { "profile": "default", "project_ref": null },
  "coverage_level": "full",
  "lifecycle_state": "confirmed",
  "health_state": "ok",
  "access_mode": "read_only",
  "confirmed_at": "2026-07-20T08:00:00Z",
  "last_success_at": "2026-07-20T08:10:00Z",
  "config_fingerprint": "sha256:..."
}
```

生命周期与健康必须分开：生命周期是 `discovered → confirmed ↔ disabled`；健康状态是 `unknown | ok | degraded | error`。路径以标准 `file:` URI 表示。[RFC 3986](https://www.rfc-editor.org/info/rfc3986/) [RFC 8089](https://www.rfc-editor.org/rfc/rfc8089.html)

#### Canonical Memory Record

```json
{
  "schema_version": "1.0",
  "record_id": "tr1_...",
  "source_id": "src_...",
  "native_id": null,
  "origin_uri": "file:///.../MEMORY.md",
  "kind": "memory",
  "scope": { "profile": "default", "project_ref": "repo-key" },
  "title": "SQLite FTS5 decision",
  "body": "保留的原始记忆正文",
  "media_type": "text/markdown",
  "locator": { "line_start": 18, "line_end": 26 },
  "source_updated_at": "2026-07-20T07:51:13Z",
  "observed_at": "2026-07-20T08:10:00Z",
  "content_hash": { "algorithm": "sha-256", "value": "..." },
  "parser": { "id": "codex-markdown", "version": "1.0.0" },
  "coverage_level": "full",
  "extensions": { "codex": {} }
}
```

关键规则：

- `record_id` 根据 `source_id + native_id/native locator` 形成稳定身份，不能直接用正文哈希，否则编辑会变成删除后新增；
- `content_hash` 只做变更检测，不证明内容可信；
- `kind` 可为 `memory | summary | profile | instruction`，不定义 `raw_chat`；原始聊天在入口策略层拒绝；
- `body` 保存源原文；任何标题、项目映射或类型推断进入单独字段并带置信度，不能覆盖原文；
- 时间使用 RFC 3339；缺少时区的源时间置空并保存原始字段，不猜时区；
- 交换格式使用 UTF-8 JSON，并固定 JSON Schema Draft 2020-12 进行验证。[RFC 3339](https://www.rfc-editor.org/info/rfc3339/) [RFC 8259](https://www.rfc-editor.org/info/rfc8259/) [JSON Schema 2020-12](https://json-schema.org/specification)

Markdown、JSON/YAML 配置和 Provider 专属 JSON 都只是 Adapter 输入；核心层不把某个 Agent 的字段结构直接暴露成公共 schema。Provider 独有字段放入命名空间化 `extensions`。

### Provider Adapter Contract

```text
descriptor()             → adapter id/version/supported source kinds
discover()               → 候选实例、路径和发现依据；默认不读正文
probe(confirmed_source)  → 版本、权限、健康与能力
scan(checkpoint?)        → 仅 capabilities.enumerate=true 时调用
search(query)            → 仅 capabilities.search=true 时调用
watch()                  → 可选失效提示流；不能直接改变索引
```

能力协商字段至少包括：`discover`、`enumerate`、`search`、`watch`、`stable_native_ids`、`source_timestamps`。由能力推导 UI 覆盖等级：

- `full`：可以完整枚举；
- `search_only`：只能查询，不能显示总数或声称完整覆盖；
- `existence_only`：只能发现/探测；
- `unsupported`：格式、权限或版本不支持。

`scan()` 与 `search()` 必须分开，避免把有限搜索结果伪装成完整目录。Adapter 输出进入中央策略检查后才能写入派生索引；出现 `raw_chat`、`whole_turn`、完整 conversation/messages 等类型时默认拒绝。

### 四个平台的接入矩阵

| Provider | 发现与源 | 连接方式 | MVP 能力 | 明确排除 |
|---|---|---|---|---|
| Codex | `CODEX_HOME/memories` 或默认目录；`MEMORY.md`、`memory_summary.md`、`raw_memories.md`、`rollout_summaries/` | 直接只读 Markdown；目录 watcher + reconcile | 文件层完整枚举 | `sessions/`、原始 rollout/transcript；内部 state DB 默认不接入 |
| Claude Code | `~/.claude/projects/*/memory/` 及用户设置的 `autoMemoryDirectory` | 直接只读 Markdown；项目映射需用户确认 | auto-memory 完整枚举 | session/transcript；`CLAUDE.md`/rules 若以后接入必须标为 instruction |
| Hermes 内建 | 活动 `HERMES_HOME` 下的 `memories/MEMORY.md`、`USER.md` | 直接只读文件，按 `§` 条目解析 | 内建记忆完整枚举 | `state.db`、session search、原始历史消息 |
| Hermes 外部 | `config.yaml` 的 active memory provider | Provider 专属 API/本地存储 Adapter | full/search-only/existence-only 逐个声明 | raw turn、messages、conversation 对象 |
| OpenClaw workspace | 各 Agent workspace 的 `MEMORY.md`、`memory/*.md`、`DREAMS.md`、imports | 直接只读 Markdown；不依赖其派生 SQLite | workspace 记忆完整枚举 | agent sessions/transcripts；插件私有库需专属 Adapter |

Codex 官方 memory pipeline 把 rollout 抽取结果写入数据库，再把选中结果同步为 `raw_memories.md` 和 `rollout_summaries/`，并由 consolidation agent 维护更高层记忆；因此这些 Markdown 是记忆工件，而不是原始聊天。[Codex Memories Pipeline](https://github.com/openai/codex/blob/main/codex-rs/core/src/memories/README.md)

Claude 官方说明 auto memory 是每个 repository 的普通 Markdown 目录，包含 `MEMORY.md` 和可选 topic files，并允许用户级 `autoMemoryDirectory`。[Claude Code Memory](https://code.claude.com/docs/en/memory)

Hermes 官方说明内建 `MEMORY.md`/`USER.md` 与一个外部 Provider 并存；其插件接口提供 prefetch、sync turn、session end 等 hooks，但没有统一 `list_all/export`，所以外部 Provider 不能承诺完整枚举。[Hermes Memory Providers](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory-providers/) [Hermes Provider Plugin API](https://hermes-agent.nousresearch.com/docs/developer-guide/memory-provider-plugin/)

OpenClaw 的 Markdown 是记忆源，其 per-agent SQLite 是派生索引；官方文档明确 watcher 可能在少数情况漏变更，因此 Tessera 不应直接复用该索引作为真相源。[OpenClaw Memory](https://docs.openclaw.ai/concepts/memory) [OpenClaw Builtin Memory](https://docs.openclaw.ai/concepts/memory-builtin)

### Watcher and Reconcile Pattern

```text
filesystem/provider hint
        ↓ debounce by source_id
mark source dirty
        ↓
scan into staging generation
        ↓
complete scan?
  ├─ yes: one SQLite transaction → upsert + delete missing + checkpoint
  └─ no: retain previous visible generation + record failure
        ↓
emit scan-state-changed; UI re-queries state
```

`notify` 官方记录了网络文件系统不发事件、macOS FSEvents 权限限制、编辑器采用截断或原子替换、大目录丢事件、Linux watch 上限等问题，并提供 PollWatcher 降级方案。因此 watcher 只能作为“失效提示”，不能成为索引正确性的来源。[notify Known Problems](https://docs.rs/notify/latest/notify/#known-problems) [notify-rs](https://github.com/notify-rs/notify)

实现约束：

1. 只监听用户确认过的 memory root，不监听整个 home；
2. 事件经 debounce 后触发受限重扫，不按 Create/Write/Delete 类型直接修改索引；
3. 用 `path + size + mtime + content_hash` 复核；读取前后 metadata 变化则丢弃并重试；
4. 定期全量 reconcile 修复漏事件；必要时切换 PollWatcher；
5. 只有完整枚举成功，才能删除本轮未出现的记录；
6. 扫描中断或单文件失败时保留上一代可用索引，显示 stale/degraded；
7. `search_only` Provider 永不根据“本次没搜到”推断删除。

### SQLite Source Integration

外部 Agent SQLite 只在没有稳定文件接口且能够证明零写入时作为可选连接器：

- 使用 `file:...?...mode=ro` 打开可能仍被 Agent 更新的数据库；
- 禁止对活跃数据库使用 `immutable=1`，因为 SQLite 会跳过锁和变更检测，文件实际变化时可能返回错误结果或 `SQLITE_CORRUPT`；
- 禁止 `nolock=1`，不改变 journal mode、schema 或 pragma；
- WAL 数据库的只读访问可能依赖现有可读 `-wal/-shm`，不能只复制主 `.sqlite` 文件，也不能为了读取而在源目录创建 sidecar；无法满足时标记 degraded/unsupported。

SQLite 官方说明 `mode=ro` 的只读语义，以及 `immutable=1` 跳过锁和变更检测的风险。[SQLite URI Filenames](https://www.sqlite.org/uri.html) [SQLite WAL Read-only](https://www.sqlite.org/wal.html#read_only_databases)

这进一步支持文件优先策略：Codex 和 Claude 的 Markdown 足以构成首版连接器；内部数据库不是 MVP 成立条件。

### System Interoperability Approaches

#### 采用：Hub-and-Adapter，而不是点对点导入

四个 Agent 不互相写入或同步。每个 Adapter 只面向 Tessera 的稳定核心契约：

```text
Codex Adapter ─────┐
Claude Adapter ────┤
Hermes Adapters ───┼─ Canonical Index ─ Query Service ─ UI/future CLI/MCP
OpenClaw Adapter ──┘
```

这样把格式漂移隔离在 Adapter 内，避免 N 个 Agent 形成 N×N 导入关系。Tessera 不成为新的权威记忆仓，也不要求源 Agent 安装插件。

#### 不采用：API Gateway、Service Mesh、ESB

这些模式解决多服务路由、服务间策略、企业消息转换等问题，而当前系统是单用户、单机、单核心进程。引入它们会扩大故障面和部署负担，不增加记忆所有权或可替换性。

### Microservices Integration Patterns

MVP 明确采用 modular monolith，而不是微服务：Adapter、Registry、Indexer、Query Service 是代码边界，不是独立部署单元。因此：

- 不需要 service discovery；
- 不需要 API gateway 或 service mesh；
- 不实现分布式 circuit breaker；Provider 专属调用只需超时、有限重试、source-level failure isolation；
- 不需要 Saga，因为 Tessera 不跨多个权威系统写事务；
- 不采用 CQRS 双模型，只有源只读与派生索引更新；
- 不采用 Event Sourcing，watcher 事件可能丢失，Agent 源才是真相源。

“不采用 CQRS/Event Sourcing”不代表不分层：查询服务、Adapter 和索引事务仍应隔离，但不制造第二套写模型或事件权威历史。简单只读/CRUD 场景通常不值得承担 CQRS 和 Event Sourcing 的投影、幂等与最终一致性成本。[CQRS Pattern](https://learn.microsoft.com/en-us/azure/architecture/patterns/cqrs) [Event Sourcing Pattern](https://learn.microsoft.com/en-us/azure/architecture/patterns/event-sourcing)

### Event-Driven Integration

内部事件只服务 UI 通知和诊断：

```text
source.discovered
source.health_changed
scan.started
scan.completed
scan.failed
index.changed
```

这些事件不是业务真相源，也不需要 Kafka/RabbitMQ 持久化。SQLite 只保存当前 projection、`scan_runs`、错误与 checkpoint。`record_id + content_hash + parser_version` 保证重试幂等；同一 `source_id` 的重复 watcher hints 合并为一个待执行扫描。

### Integration Security Patterns

#### 用户确认和路径白名单

1. 自动发现阶段只返回 candidate metadata；默认不读取正文；
2. 用户确认后，Rust 保存 canonicalized source root 与 `source_id`；
3. 后续命令只接受 ID，不接受任意路径；
4. 每次读取重新 canonicalize 目标并验证仍在 confirmed root 内；
5. symlink 越界、权限变化和格式变化转为 connector error，不自动扩大路径；
6. Tessera 唯一可写区域是自己的 app-data/index 目录。

React WebView 不需要任何 `fs:*` 权限。Tauri Capability/Permission 可以把命令限制到固定 window 和 scope；多个 Capability 命中同一 window 时权限会合并，因此主窗口应只有一套最小权限配置。Tauri 默认只允许 bundled code 访问 API，应保持远程 URL 无 Capability。[Tauri Capabilities](https://v2.tauri.app/security/capabilities/) [Tauri Permissions](https://v2.tauri.app/security/permissions/)

一个实施期必须验证的细节是：应用通过 `invoke_handler` 注册的自定义 commands 默认可被所有应用 window/webview 调用，需要通过 `AppManifest::commands` 纳入 Capability 管理，不能只配置插件权限就认为已经隔离。[Tauri Capabilities](https://v2.tauri.app/security/capabilities/)

#### 凭据和外部 Provider

- Registry 不保存明文 token；只保存 Keychain/environment/config reference；
- Provider Adapter 只请求最小只读 scope；
- 超时、限流或认证失败只降级对应 source，不影响其他连接器；
- 搜索结果和错误日志不回显 secret；
- 默认不持久化用户查询正文；
- 对 Hermes 外部 Provider，若 API 不能区分 memory 与 conversation/raw turn，则不索引并显示能力缺口。

OAuth/JWT、mTLS 和 API key rotation 仅在未来远端 Provider 或 HTTP MCP 出现时适用；本地文件连接器不应为了形式统一而引入认证服务器。MCP HTTP transport 若未来启用，需要遵守官方授权规范和 Origin/DNS rebinding 防护；stdio 模式则从本地环境获取凭据。[MCP Authorization](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization) [MCP Transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)

### Cross-Integration Analysis

| 关键问题 | 统一策略 | 不能统一的部分 |
|---|---|---|
| 来源发现 | candidate → user confirmation → confirmed source | Agent/profile 路径与配置规则 |
| 数据读取 | read-only Adapter + canonical envelope | 文件、SQLite、Provider API 的具体协议 |
| 完整性 | coverage level 明示 | search-only Provider 不存在可靠总数 |
| 变化检测 | watcher hint + periodic reconcile + hash | Provider API 的 checkpoint/pagination 语义 |
| 查询 | 单一 Query Service 与过滤模型 | Provider 原生 search 的排序和召回 |
| 安全 | ID-based commands、path allowlist、失败隔离 | 外部 Provider 的凭据和授权模型 |
| 可替换性 | transport-neutral schema、CLI/MCP 可后加 | Agent 是否支持 MCP、CLI 或插件 |

最小闭环是：发现 Codex/Claude/Hermes/OpenClaw 的候选源 → 用户确认 → 完整扫描或能力降级 → 统一搜索 → 查看原文和来源 → 修改源文件后自动/手动 reconcile。它不需要 Agent 配合，也不改变任何 Agent 的记忆实现。

### Quality Assessment

| 判断 | 置信度 | 主要缺口 |
|---|---|---|
| 文件型 Adapter + Canonical Index 可行 | 高 | 需要固定多版本匿名 fixture 防止格式漂移 |
| Tauri Command/Channel/Event 分工 | 高 | 实施时验证自定义 command capability 与窗口隔离 |
| watcher 只能做 hint、reconcile 保证一致性 | 高 | 各平台删除/原子替换和 PollWatcher E2E 测试 |
| Hermes 外部 Provider 可统一枚举 | 低 | 官方接口没有 list/export；必须逐 Provider 实测 |
| OpenClaw workspace 文件可完整枚举 | 高 | 非默认插件私有数据面仍需逐个 Adapter |
| MCP stdio 适合作为未来 Agent 查询入口 | 中高 | 四个平台具体版本的 MCP client 支持需实施时验证 |
| 纯只读打开所有 Agent SQLite | 中低 | 活跃 WAL、sidecar 与 schema 漂移可能迫使连接器降级 |

**第 3 步集成判断：** Tessera 的护城河不是制造一个“万能 Memory API”，而是把不可统一的 Provider 差异诚实封装为能力契约，并让用户在同一个只读界面看见覆盖范围、来源和健康状态。MVP 的系统互操作是 Hub-and-Adapter；未来 Agent 访问优先采用 CLI JSON 与 MCP stdio，而不是先部署常驻本地 HTTP 服务。

## Architectural Patterns and Design

### System Architecture Patterns

Tessera 采用 **本地优先的模块化单体 + 六边形 Adapter 边界**。它不是把四种 Agent 重新实现一遍，而是在本地建立一个只读控制面：外部 Agent 记忆通过 Adapter 进入统一的派生索引，UI 与未来 CLI/MCP 只调用 Query Service。

```text
React WebView
    │
Tauri IPC Boundary
    │
Rust Application Core
    ├─ Source Registry
    ├─ Access Policy / Path Guard
    ├─ Adapter Registry
    ├─ Scan Orchestrator
    ├─ Reconciler
    ├─ Canonical Index
    └─ Query Service
         │
SQLite + FTS5
         ▲
         │ read-only adapters
Codex / Claude Code / Hermes / OpenClaw
```

Tauri 的多进程模型天然给出两个信任域：Core process 拥有操作系统权限并负责全局状态、数据库连接和业务逻辑，WebView 只渲染 UI 并通过 IPC 请求受控能力。官方明确建议不在前端处理 secret，并尽量把业务逻辑放在 Core 以缩小攻击面。[Tauri Process Model](https://v2.tauri.app/concept/process-model/)

模块职责如下：

| 模块 | 负责 | 明确不负责 |
|---|---|---|
| Source Registry | 候选来源、用户确认、能力和健康状态 | 保存源记忆正文或明文凭据 |
| Access Policy / Path Guard | 路径白名单、来源确认、聊天排除策略 | Provider 格式解析 |
| Provider Adapter | 发现、探测、枚举/搜索、规范化 | 索引事务、UI 或跨 Provider 决策 |
| Scan Orchestrator | 调度、限流、取消、失败隔离 | 解析具体格式 |
| Reconciler | watcher hints、周期校验、哈希比较 | 把 watcher 事件当作事实 |
| Canonical Index | 统一记录、FTS5、扫描状态 | 成为新的权威记忆仓 |
| Query Service | 搜索、过滤、分页、来源回溯 | 直接读取任意文件或绕过 Policy |
| React UI | 展示、筛选、确认和诊断 | 文件系统、SQL、Provider 凭据 |

未来 CLI、MCP 或浏览器 API 都只能复用 Query Service，不能绕过 Source Registry、Access Policy 和 Adapter。

### Design Principles and Best Practices

#### 1. Agent 可替换，记忆资产不可被 Tessera 劫持

Agent 源文件或 Provider 是唯一真相；Tessera 只保存可删除、可重建的 projection。Ink & Switch 对 local-first 的核心论述是网络可选、隐私、长期可访问和用户保有最终控制权；这些原则与本产品的本地只读定位一致。[Local-first Software](https://www.inkandswitch.com/essay/local-first/)

Tessera 当前不需要 CRDT。CRDT 解决多副本协同写入和冲突合并，而当前产品不写回 Agent，也不维护多个可写权威副本。引入 CRDT 会优化一个尚不存在的问题。

#### 2. Read-only by construction

只读不能只是一条产品声明：

- Adapter trait 不提供 write/delete/update；
- WebView 不获得文件系统权限；
- Commands 只接受受控 ID；
- 外部 SQLite 使用只读连接或明确降级；
- Tessera 唯一可写区域是自己的 app-data/index；
- 测试验证源目录在 scan 前后没有文件、mtime 或 hash 变化。

#### 3. Capability honesty

完整枚举、仅搜索、仅探测必须是显式能力，不允许 UI 通过模拟总数或模糊文案掩盖差异。统一的是调用契约、来源和健康状态，不是 Provider 的真实能力。

#### 4. Idempotent and rebuildable

相同输入、Adapter 版本和 parser 版本应产生相同 canonical records。所有扫描可安全重试，索引可完整重建，失败不影响源数据。

#### 5. Preserve evidence before inference

保存原文、原路径、原始 locator 和 content hash。标题提取、项目映射、分类等推断不能覆盖原始证据，并需带推断来源与置信度。

#### 6. Graceful degradation

一个来源失败只降低该来源的健康状态；上一代可用索引继续显示并明确标记 stale。系统不能因为 Hermes Mem0 认证失败就让 Codex 和 Claude 的记忆也不可用。

#### 7. Versioned boundaries

Registry schema、canonical record、Adapter 和 parser 都独立版本化。Provider 格式变化由对应 Adapter 吸收，不通过修改公共 schema 破坏其他连接器。

### State Machines and Consistency Model

来源生命周期、连接器健康和单次扫描分别建模：

```text
Source lifecycle: discovered → confirmed ↔ disabled

Health: unknown → ok
               ↘ degraded
               ↘ error

Scan: queued → scanning → committing → completed
                  ↘ failed
                  ↘ cancelled
```

例如，一个来源仍是 `confirmed`，但本轮 scan 可以 `failed`。UI 继续展示上一成功 generation，同时显示 stale/error；不能因为一次失败让全部历史结果突然消失。

每个来源使用 generation-based indexing：

1. scan 结果写入 staging generation；
2. 所有条目完成解析后，在单个 SQLite 事务中切换 active generation；
3. 失败或取消时丢弃 staging，保留上一成功 generation；
4. 只有完整枚举成功，才能移除本轮未出现的记录；
5. `search_only` Provider 永不根据“没有搜到”推断删除；
6. parser 版本升级触发受控重解析，但不修改源。

SQLite 的事务具有原子提交语义；WAL 允许读写并发，并给读取事务提供 snapshot isolation，适合 UI 持续查询与后台构建新 generation 并行进行。[SQLite Atomic Commit](https://www.sqlite.org/atomiccommit.html) [SQLite WAL](https://www.sqlite.org/wal.html) [SQLite Isolation](https://www.sqlite.org/isolation.html)

### Scalability and Performance Patterns

MVP 是单用户本地桌面产品，真实扩展方向是 **更多来源和更多文件**，不是更多服务实例。因此采用纵向优化和有界并发：

- 不同 source 可有限并发扫描；同一个 source 的扫描严格串行；
- watcher hints 按 `source_id` 合并，避免重复排队；
- 文件先比较 size/mtime，可能变化时再计算内容 hash；
- 解析器流式读取大文件，设单文件与单来源的大小/时间预算；
- search 使用过滤条件、FTS5 `rank`、`LIMIT` 和稳定 cursor pagination；
- 首次扫描和全量重建使用 Channel 报告进度，避免 UI 阻塞；
- 搜索默认返回 snippet，完整正文由 `get_memory` 按需读取索引记录；
- 不在列表页一次返回全部正文。

FTS5 的 `rank` 列在带 LIMIT 或提前终止的排序查询中可以比直接调用 `bm25()` 更快；`snippet()` 可返回匹配上下文。[SQLite FTS5 Ranking](https://www.sqlite.org/fts5.html#sorting_by_auxiliary_function_results)

FTS5 `optimize` 会合并索引 b-tree，使空间最小、查询最快，但官方也说明整个操作可能耗时较长。因此只在空闲维护或诊断确认碎片化后执行，不能每轮 scan 都运行。[SQLite FTS5 optimize](https://www.sqlite.org/fts5.html#the_optimize_command)

不设计分片、负载均衡、分布式缓存或水平扩展。只有真实基准证明单文件 SQLite 无法满足规模和延迟时，才重新评估存储。

### Integration and Communication Patterns

架构上保持 transport-neutral：

```text
React UI ─ Tauri transport ─┐
CLI JSON ─ process stdout ──┼─ Query Service ─ Policy ─ Index
MCP stdio ─ JSON-RPC ───────┘
```

MVP 只实现 Tauri transport。Query Service 不包含 Tauri 类型，使以后增加 CLI/MCP 时不重写业务逻辑。所有 transport 都只能访问相同的只读查询和 health API。

核心内部使用有界任务队列而非消息 broker。Scan Orchestrator 对每个 source 去重、控制并发并支持取消；Reconciler 只提交 `ReconcileRequested(source_id)`，不能直接写索引。

Provider 调用采用 timeout、有限重试和退避。这里不需要分布式 circuit breaker，但需要 source-level failure isolation 和下一次允许重试时间。

### Security Architecture Patterns

Tauri 的安全模型明确区分拥有完整系统资源的 Core 与只能通过 IPC 获得暴露能力的 WebView，因此所有路径检查、Adapter、数据库和凭据访问必须在 Rust Core 内。[Tauri Security](https://v2.tauri.app/security/)

安全控制包括：

- WebView 不处理 Provider token、文件句柄或 SQLite connection；
- Commands 只接受 `candidate_id`、`source_id`、`record_id`；
- 每次读取重新 canonicalize 目标并验证位于 confirmed root；
- Capability 只授予固定主窗口，不使用通配远程来源；
- 主窗口避免重叠多个 Capability，防止权限集合合并；
- CSP 只加载 bundled assets，不允许 CDN script 或远程网页；
- parser 输入按不可信内容处理，Markdown 只渲染安全子集；
- 日志保存 source、时间、状态和错误类型，默认不保存记忆正文、凭据和搜索词；
- 任何导出都需要用户明确选择目标和内容范围。

Tauri 官方 CSP 指南建议避免远程脚本，并把 CSP 限制到受信任来源；本产品可以进一步保持 `default-src 'self'` 的离线壳。[Tauri CSP](https://v2.tauri.app/security/csp/)

跨平台 symlink/TOCTOU 需要单独 threat test。MVP 使用 canonical path + root containment 检查；若风险评估要求更强，再采用平台目录句柄相对打开和禁止跟随 symlink。

### Data Architecture Patterns

数据分四类表/索引：

```text
source_registry
  source identity, confirmation, capability, lifecycle, health

memory_records
  canonical records, provenance, generation, parser version

memory_fts
  title/body full-text projection

scan_runs / connector_errors
  progress, checkpoint, counts, errors and stale reason
```

设计原则：

- `record_id` 是稳定身份，`content_hash` 是内容版本；
- `source_id + native_id/native locator` 决定 identity；
- scope、kind 和 coverage 是明确字段，不塞进正文；
- Provider 独有数据放 namespaced extensions；
- schema migration 与 parser reindex 分开：前者迁移 Tessera 数据结构，后者重建 Provider projection；
- Source Registry 和用户设置值得备份，索引本身可重建；
- 不把原 Agent SQLite attach 到 Tessera index 中做跨库 join；读取、规范化后再写入自己的数据库。

对于源删除，只有一次完整成功 reconcile 才能确认缺失；在临时卸载磁盘、权限丢失或 Provider 超时情况下，记录只进入 stale，不进入 removed。

### Failure Isolation and Recovery

| 故障 | 系统行为 | 保留内容 |
|---|---|---|
| 单文件无法解析 | 标记 item error，source degraded | 其他文件和上一条有效记录 |
| Source root 消失 | 停止 scan，source degraded/stale | 上一 active generation |
| Provider 认证/限流失败 | 退避并显示下一重试信息 | 其他 Provider 和旧缓存 |
| Adapter panic/崩溃 | 捕获任务失败，不提交 staging | 源文件和上一 generation |
| schema 不兼容 | 禁止该 Adapter 写新 generation | 其他连接器正常运行 |
| Tessera index 损坏 | 停止查询或进入恢复模式，允许重建 | Agent 源数据不受影响 |
| migration 失败 | 不启动 watcher/index writer，保留备份 | 原索引和全部 Agent 源 |

健康页需要展示：发现依据、确认路径、Adapter/parser 版本、coverage、watch mode、last successful scan、last error、stale age 和手动重扫入口。

### Architectural Decision Records

| ADR | 决策 | 主要理由 |
|---|---|---|
| ADR-001 | Tauri/Rust 模块化单体 | 单机拓扑、最小部署和统一安全边界 |
| ADR-002 | Agent 源是唯一真相 | 用户所有权与 Agent 可替换性 |
| ADR-003 | Adapter 能力协商 | Provider 无法提供等价接口 |
| ADR-004 | staging generation + 原子切换 | 避免半套索引和失败清空 |
| ADR-005 | MVP 只用 SQLite FTS5 | 减少隐私、模型和运行时依赖 |
| ADR-006 | MVP 不开放 HTTP 端口 | 减少认证、CORS 和本地攻击面 |
| ADR-007 | Query Service transport-neutral | 为 CLI/MCP 留扩展点而不提前实现 |
| ADR-008 | instruction/memory/summary/profile 分型 | 避免把不同语义混成“记忆” |
| ADR-009 | 知识库以后复用基础设施但独立 Adapter | 不扩大首版范围，不阻断后续方向 |
| ADR-010 | 不使用 CRDT/Event Sourcing | 没有协同写入或 Tessera 自有事件真相源 |

### Deployment and Operations Architecture

MVP 首先支持 macOS 本地安装：

- 应用启动顺序：配置加载 → schema migration → Registry 验证 → index integrity check → watcher/reconcile；
- migration 前备份 Tessera 自有数据库，不备份或修改 Agent 源；
- app version、schema version、Adapter/parser version 分开记录；
- 提供手动重扫、完整重建索引、导出诊断报告；
- 默认不启用遥测、远程日志或网络请求；
- 正式发布使用签名和 notarization；更新包也必须验证签名；
- macOS 稳定后再用同一 fixture suite 验证 Windows 路径、WebView2 和 Linux WebKitGTK 差异。

Tauri 官方提供平台 installer/bundle；macOS 直接分发 DMG 需要 code signing 和 notarization。[Tauri Distribution](https://v2.tauri.app/distribute/)

**第 4 步架构判断：** Tessera 最合适的架构不是“通用记忆平台微服务”，而是一个有明确安全边界、可重建 projection 和 Adapter 能力契约的本地模块化单体。扩展性来自稳定的模块/transport 接口，而不是提前拆进程、上云或引入分布式基础设施。

## Implementation Approaches and Technology Adoption

### Technology Adoption Strategies

技术采用使用渐进式纵向切片，而不是一次性完成四个平台和全部 Provider。每一阶段都必须产生一个可运行、可验证的闭环，并优先处理可能推翻整体方案的风险。

#### Phase 0：技术风险验证

只验证四个关键假设：

- Tauri 2 在目标 macOS 环境完成开发构建与最小安装包；
- `rusqlite bundled` 确认包含 FTS5/trigram；
- 自定义 Tauri Command 能被 Capability 正确限制；
- 对真实 Codex 目录执行发现/扫描后，源目录文件集合、mtime、size 和 hash 均无变化。

任一项失败时先调整技术决策，不进入正式 UI 开发。Tauri 官方提供项目模板、平台前置依赖和开发启动流程，可用于构建最小 spike。[Tauri Create Project](https://v2.tauri.app/start/create-project/) [Tauri Prerequisites](https://v2.tauri.app/start/prerequisites/)

#### Phase 1：Codex 端到端纵向切片

```text
发现 Codex
→ 用户确认
→ 扫描 Markdown
→ 写入派生索引
→ 搜索
→ 展示结果卡
→ 打开来源
→ 手动重扫
```

这一步同时建立最小 Adapter、Registry、Canonical Record、SQLite schema、Query Service、Tauri Commands 和 Explorer UI。先用一个 Provider 证明整体闭环，避免先写大量无法一起运行的抽象层。

#### Phase 2：Claude Code

增加默认 projects memory、`autoMemoryDirectory`、项目映射确认，以及 `CLAUDE.md`/rules 与 auto-memory 的分类隔离。重点覆盖多个 repository、worktree、自定义目录和无法反向映射的 project key。

#### Phase 3：Hermes 与 OpenClaw 文件记忆

增加 Hermes 多 `HERMES_HOME` profile、`MEMORY.md`/`USER.md` 的 `§` 解析，以及 OpenClaw 多 Agent workspace、daily memory、`DREAMS.md` 和 imports。OpenClaw 未安装时支持官方结构的手动路径连接，但不能声称完成本机实例验证。

#### Phase 4：增量更新与故障恢复

增加 watcher、debounce、周期 reconcile、generation commit、stale/degraded 状态、完整重建，以及路径消失、权限变化、原子替换和 watcher 丢事件测试。

#### Phase 5：Hermes Mem0 专属 Adapter

这是独立风险项目，不属于“通用 Hermes Adapter”：

- 核验当前 Mem0 实例是否支持稳定 list、pagination、get 和类型区分；
- 只能搜索时实现 `search_only`；
- 无法可靠排除 raw turn/conversation 时不索引；
- 禁止 Tessera 直接并发打开 Hermes 使用中的 embedded Qdrant；
- Provider 升级必须通过真实实例 contract test。

#### Phase 6：发布硬化

完成 macOS 签名与 notarization、schema migration、索引备份/恢复、安装包 smoke test、依赖审计、诊断报告脱敏与更新失败恢复。

### Development Workflows and Tooling

首版保持一个 Rust crate，不提前拆 workspace：

```text
src-tauri/src/
  domain/
    source.rs
    memory.rs
    capability.rs
    scan.rs
  adapters/
    codex.rs
    claude.rs
    hermes_builtin.rs
    openclaw.rs
    mem0.rs
  application/
    discovery.rs
    scanning.rs
    reconciliation.rs
    querying.rs
  infrastructure/
    sqlite.rs
    migrations.rs
    watcher.rs
    credentials.rs
  security/
    path_guard.rs
    content_policy.rs
  commands/
    sources.rs
    search.rs
    scan.rs

src/
  features/
    onboarding/
    search/
    sources/
    health/
  lib/
    ipc/
    types/

fixtures/
  codex/
  claude/
  hermes/
  openclaw/

schemas/
  canonical-memory-record.schema.json
  source-registry.schema.json
```

只有 CLI/MCP 确实需要复用核心时，再抽取独立 `tessera-core` crate。单次使用的模块不提前抽象成插件系统。

每次变更的本地验证链：

```text
cargo fmt --check
→ cargo clippy
→ cargo test
→ frontend typecheck
→ Vitest
→ Tauri build
```

`cargo test` 原生支持 unit、integration 和 documentation tests；Clippy 用于捕获正确性、可疑代码、复杂度和性能问题。[Cargo Test](https://doc.rust-lang.org/cargo/commands/cargo-test.html) [Clippy](https://doc.rust-lang.org/stable/clippy/)

CI 分两层：

- 每次提交：macOS 执行 format、lint、Rust/前端测试、fixture contract tests 和 debug build；
- 发布候选：macOS ARM/Intel 安装包构建、签名、安装与 smoke test；Windows/Linux 在正式支持前只做编译和核心 fixture 验证。

Tauri 官方 Action 可构建 macOS、Linux、Windows 原生应用并上传 release artifacts。[Tauri Action](https://github.com/tauri-apps/tauri-action)

### Testing and Quality Assurance

#### Parser fixture tests

每个平台保存匿名化、多版本 fixture，至少覆盖正常文件、空文件、中文/Unicode、YAML frontmatter、损坏 Markdown、超长行、未知字段、格式升级、symlink 和原子替换。输出使用 golden canonical records 验证。

#### Source mutation invariant

```text
scan 前记录文件树、mtime、size、hash
→ 执行 discover/probe/scan/search
→ scan 后重新记录
→ 必须完全一致
```

这是 MVP 最重要的发布门禁，而不是一条无法证明的“只读”声明。

#### Index consistency tests

- 相同输入重复扫描不产生重复记录；
- 删除索引后重建得到相同 record IDs 和 hashes；
- 扫描中断不暴露 staging generation；
- 单文件失败不清空上一 generation；
- 只有完整成功 scan 才能确认删除；
- search-only 来源不会产生伪造的全量计数或 tombstone。

#### Security tests

- `../` 路径越界与 URI 编码变体；
- symlink 越界、确认后 root 被替换；
- Markdown 中的 script、HTML、危险链接；
- 前端伪造 source/path/SQL；
- 未授权 window 调用 Command；
- 诊断日志泄露正文、路径外内容或 token；
- 外部 SQLite/WAL 在只读操作后产生新 sidecar。

#### UI 和 E2E

- Vitest：筛选、coverage、stale/error、来源卡片；
- Tauri mock runtime：Command 参数、错误映射与状态流；
- WebDriver：发现 → 确认 → 搜索 → 来源回溯的黄金路径；
- 安装包测试：首次启动、索引重建、升级后 migration。

Tauri 官方支持 mock runtime 的 unit/integration testing 和 WebDriver E2E；当前 WebdriverIO Tauri service 支持 Windows、Linux 和 macOS，并可在浏览器模式下 mock IPC。[Tauri Tests](https://v2.tauri.app/develop/tests/) [Tauri WebDriver](https://v2.tauri.app/develop/tests/webdriver/)

#### 搜索评测

建立匿名化查询集，覆盖 exact ID/path、中文短语、中英混合、project/provider filter、同义表达和无结果查询。持续记录 Recall@k、MRR、无结果率和人工相关性。先建立 FTS5 基线，再用同一数据证明是否需要语义检索。

### Deployment and Operations Practices

应用启动顺序：

```text
加载设置
→ 迁移 Tessera schema
→ SQLite integrity check
→ 验证 confirmed sources
→ 加载上一 active generation
→ 启动 watcher
→ 后台 reconcile
```

运维面只提供本地、可解释的控制：连接器健康、scan duration/counts、Adapter/parser 版本、watcher mode、last error、stale age、index size、手动重扫和重建索引。

默认不记录记忆正文、搜索词、token，不发送遥测或远程日志。索引损坏时允许完整重建；Source Registry 和用户设置需要备份。正式 macOS 分发需要签名与 notarization。[Tauri macOS Signing](https://v2.tauri.app/distribute/sign/macos/)

发布流程：

1. lockfile 与 schema/Adapter version 固定；
2. 全部质量门通过；
3. 构建签名安装包；
4. 在干净用户环境安装；
5. 运行文件型 Provider smoke fixture；
6. 验证无外部网络和无源目录变更；
7. 发布 draft，经人工确认后正式发布。

### Team Organization and Skills

最小实施能力包括：

- Rust：trait、错误模型、async task、SQLite、文件系统安全；
- Tauri：IPC、Capability、窗口生命周期、打包与签名；
- React/TypeScript：搜索、筛选和连接器状态界面；
- 数据工程：幂等扫描、checkpoint、schema migration、FTS5；
- 测试：fixture contract、文件系统故障、E2E；
- Provider 研究：四个平台的配置、格式和版本漂移。

开发责任可按稳定边界划分为 Core/Adapter、UI/IPC、fixtures/QA，但架构和数据契约必须由一个 owner 统一维护。MVP 不需要微服务、Kubernetes、云数据库、模型训练或专职 MLOps。

### Cost Optimization and Resource Management

- 文件连接器不产生云 API 费用；
- SQLite 不需要独立数据库服务；
- 不使用 embedding，避免模型、向量存储、网络和隐私成本；
- 主要外部成本来自代码签名、CI 构建和用户自选外部 Provider；
- 限制扫描并发、文件大小、Provider 超时和查询 limit，避免本地资源失控；
- 不直接复用 OpenClaw/Hermes 的派生索引作为 Tessera 权威索引，避免隐藏的 schema/version 耦合；
- Release pipeline 只在 tag/draft release 时执行完整多架构构建，日常 CI 用核心测试和单目标 debug build。

供应链要求：提交 `Cargo.lock` 和前端 lockfile；固定 Rust/Node/Tauri 版本；CI 执行 `cargo audit`；GitHub Actions 使用固定版本或 commit SHA；新增依赖说明用途、维护状态和许可证。RustSec 官方数据库可通过 `cargo-audit` 检查锁文件中的已知漏洞。[RustSec](https://rustsec.org/)

### Risk Assessment and Mitigation

| 风险 | 影响 | 缓解方式 |
|---|---|---|
| Rust/Tauri 学习成本 | 延迟首个闭环 | Phase 0 spike；先完成 Codex vertical slice |
| Agent 格式持续变化 | Parser 失效或错误归类 | 多版本匿名 fixture、Adapter/parser 独立版本 |
| Claude project 映射错误 | 记忆归错项目 | 用户确认，保留原 project key，不猜路径 |
| watcher 丢事件 | 结果过期 | 周期 reconcile、PollWatcher、手动重扫 |
| 源 SQLite 产生 sidecar | 破坏零写入承诺 | 文件优先；无法证明零写入时降级 |
| Markdown/XSS | WebView 被攻击 | 安全渲染、严格 CSP、无远程内容 |
| Hermes Provider 无完整导出 | UI 误导用户 | coverage 等级，不伪造总数/完整性 |
| 索引 migration 失败 | 应用不可查询 | migration 前备份、自有 DB 可重建 |
| 外部 Provider 凭据泄露 | 隐私与账户风险 | Keychain/reference、日志脱敏、最小 scope |
| 范围失控 | MVP 无法交付 | 不做知识库、写回、语义搜索、云同步、Agent 注入 |
| 四平台并行开发 | 长期没有闭环 | Codex → Claude → Hermes/OpenClaw 顺序加入 |

## Technical Research Recommendations

### Implementation Roadmap

推荐顺序为：

1. Phase 0 技术风险 spike；
2. Codex 完整 vertical slice；
3. Claude Code Adapter 与项目映射；
4. Hermes 内建与 OpenClaw workspace；
5. watcher/reconcile/恢复与健康页；
6. Hermes Mem0 专属研究和 Adapter；
7. macOS 分发硬化；
8. 基于真实指标决定语义搜索、CLI/MCP 与知识库连接器。

每个阶段的退出标准都是可运行功能和验证证据，不以“代码已写完”作为完成定义。

### Technology Stack Recommendations

| 用途 | 推荐 | 说明 |
|---|---|---|
| Desktop/Core | Tauri 2 + Rust | 本地安全边界和原生分发 |
| UI | React + TypeScript + Vite | 搜索、来源和健康视图 |
| Index | SQLite + FTS5 | 单文件、事务化、可重建 |
| SQLite driver | rusqlite bundled | 固定 SQLite/FTS5 能力 |
| Watch | notify + debouncer/PollWatcher | hint + reconcile，不承担真相 |
| Rust tests | cargo test | unit、integration、doc tests |
| UI tests | Vitest | Vite/TS 组件和逻辑 |
| Desktop E2E | WebdriverIO Tauri | 黄金路径与安装包 smoke |
| Supply chain | Cargo.lock、frontend lockfile、cargo-audit | 可复现与漏洞审计 |

### Skill Development Requirements

优先补足顺序：

1. Tauri Capability、自定义 Commands 与 Core/WebView trust boundary；
2. Rust 文件系统安全、SQLite transactions/WAL/FTS5；
3. fixture-driven parser development；
4. source generation、reconcile 和失败恢复；
5. macOS 签名、notarization 和 installer testing；
6. Mem0 API/数据模型，只在基础闭环完成后学习。

### Success Metrics and KPIs

以下是验收标准，不是假设的当前数据：

- 所有文件型 Adapter contract tests 中源目录零变化；
- 每条文件型搜索结果都有 Provider、source、路径、locator 和 hash；
- 删除 Tessera index 后可得到相同稳定 record IDs；
- 单个 Adapter 失败时其他来源仍可搜索；
- `search_only` 来源不显示虚假总数或“完整同步”；
- file-only 模式没有外部网络请求；
- 日志与诊断报告不包含记忆正文或凭据；
- 安装包通过发现、扫描、搜索、来源回溯和重建索引 smoke test；
- 搜索延迟、冷扫描时间、内存和索引体积通过固定匿名语料持续测量；具体阈值由首轮基准确定，不凭空设定；
- FTS5 与任何未来语义检索使用同一评测集比较，只有显著增益才允许增加依赖。

**第 5 步实施判断：** 从 Codex 纵向切片开始，再顺序增加 Claude、Hermes/OpenClaw，是风险最低且最快形成证据的路径。项目成功的首要指标不是支持多少 Provider，而是能否证明零写入、来源可追溯、失败隔离和覆盖诚实。

## Final Research Synthesis

### Strategic Decision

Tessera 应定位为 **用户拥有的跨 Agent 本地记忆联邦层**，而不是新的 Agent、聊天归档器或统一记忆写入引擎。Agent 是可替换的执行介质，源记忆和知识资产才是长期保留对象。这个定位与 local-first 的数据所有权原则一致，也与 OpenClaw 已提供的 Codex/Claude Code 记忆导入功能形成清晰差异：OpenClaw 通过复制服务于自身 Agent，Tessera 则通过实时只读索引服务于用户对全部记忆资产的观察、搜索和迁移判断。[Local-first Software](https://www.inkandswitch.com/essay/local-first/) [OpenClaw Memory](https://docs.openclaw.ai/concepts/memory)

### MVP Product Boundary

首版包含：

- 自动发现候选记忆源，并由用户确认接入；
- 接入 Codex、Claude Code、Hermes 内建文件和 OpenClaw workspace Markdown；
- 跨来源全文搜索、Agent/项目筛选和原始位置回溯；
- 展示 Provider、项目、路径、locator、更新时间、内容哈希和覆盖级别；
- 展示连接器健康、最近扫描、失败原因、手动重扫和完整重建；
- 以 generation-based staging 和原子切换保证失败时保留上一可用索引。

首版明确排除：

- 原始聊天记录；
- Agent 记忆写回、编辑和自动冲突合并；
- 云上传、多设备同步和远程遥测；
- AI 摘要、向量数据库和未经基准证明的语义搜索；
- 对所有 Hermes 外部 Provider 的虚假通用化；
- 知识库统一管理。知识库未来复用 Adapter、Registry 和 Query 基础设施，但保持独立领域模型。

### Interoperability and Coverage Confidence

| 范围 | 置信度 | 判断依据 |
|---|---|---|
| Codex 文件记忆 | 高 | 官方流水线、文件结构和本机只读审计相互印证 |
| Claude Code auto memory | 高 | 官方明确目录、作用域和 Markdown 结构，本机存在真实样本 |
| Hermes 内建文件 | 高 | 内建 `MEMORY.md`/`USER.md` 始终启用 |
| Hermes 外部 Provider | 低至中 | Provider 的枚举、搜索、导出能力不同，必须专属适配 |
| OpenClaw workspace | 中高 | 官方明确 Markdown 结构与 SQLite 派生索引，但本机尚无实例验证 |
| OpenClaw 插件后端 | 中 | 需要按插件能力另行适配，不能由 workspace 结论外推 |

Codex 官方实现显示其记忆由会话级提取和全局整合两阶段生成；Claude Code 官方区分人工维护的 `CLAUDE.md` 与项目级 auto memory；Hermes 官方确认内建文件记忆与一个可选外部 Provider 并存。这些差异证明 capability-based Adapter 比“统一文件格式”假设更可靠。[Codex Memories Pipeline](https://github.com/openai/codex/blob/main/codex-rs/core/src/memories/README.md) [Claude Code Memory](https://code.claude.com/docs/en/memory) [Hermes Memory Providers](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory-providers/)

### Security, Performance, and Operational Gates

进入可发布状态前必须同时满足：

1. **零源修改**：Adapter contract test 比较扫描前后的文件集合、大小、mtime 和内容哈希；外部 SQLite 只允许只读打开，无法证明不产生 sidecar 时降级为文件读取或 unsupported。
2. **路径安全**：所有路径先 canonicalize，再校验确认根目录；阻断 symlink escape、path traversal 和任意路径命令。
3. **最小权限**：WebView 不获得文件系统、shell 或凭据权限，只调用 ID 化的 Tauri commands；使用严格 CSP 和安全 Markdown 渲染。[Tauri Capabilities](https://v2.tauri.app/security/capabilities/) [Tauri Permissions](https://v2.tauri.app/security/permissions/)
4. **覆盖诚实**：`search_only` 和 `existence_only` 不显示虚假总数或“完整同步”。
5. **失败隔离**：单个 Adapter 超时、格式错误或 Provider 不可用时，其他来源继续可查询。
6. **可重建性**：删除派生索引后，固定 fixture 应生成相同稳定 record IDs 和可解释来源。

SQLite FTS5 足以承担首版全文检索；冷扫描时间、查询延迟、内存和索引体积应使用固定匿名语料测量，阈值由首轮基准确定，不在缺乏数据时编造。只有同一评测集证明语义检索带来显著增益后，才增加 embedding 和向量索引。[SQLite FTS5](https://www.sqlite.org/fts5.html)

### Delivery Roadmap and Risk Control

1. Phase 0：验证 Tauri 构建、bundled FTS5、Capability 和零源修改测试。
2. Phase 1：完成 Codex 发现、确认、扫描、索引、搜索、回溯的纵向闭环。
3. Phase 2：接入 Claude Code，并解决 repository/worktree 到 project memory 的映射。
4. Phase 3：接入 Hermes 内建文件和 OpenClaw workspace。
5. Phase 4：补齐 watcher、周期 reconcile、generation 切换、失败恢复和健康页。
6. Phase 5：根据真实使用需求开发 Hermes Mem0 专属 Adapter。
7. Phase 6：完成安装包、签名、notarization、供应链审计和干净环境 smoke test。

每阶段都以可执行验证证据退出，不以“代码已经写完”为完成标准。最主要的范围风险是过早加入知识库、写回、语义搜索和云同步；最有效的控制方式是保持一个 Rust core crate 和一个端到端纵向切片，待稳定边界出现后再拆分。

### Future Outlook

只读浏览闭环稳定后，可按证据依次考虑：

- CLI JSON 输出，让其他本地工具查询 Tessera；
- MCP stdio server，让 Agent 在授权后检索跨来源记忆；
- 独立的 Knowledge Source Adapter 和知识库搜索域；
- 基于固定评测集的混合检索；
- 可导出、可迁移的用户自有 canonical archive。

任何写回、自动合并或跨设备同步都会改变 Tessera 的信任模型和冲突模型，必须作为新的产品阶段重新设计，而不是作为 Adapter 的顺手扩展。

### Methodology and Source Verification

本研究采用三层证据：官方文档/官方仓库、本机真实安装只读审计、关键运行时能力验证。结论只在证据允许的范围内成立：Codex、Claude Code 和 Hermes 已有本机数据支撑；OpenClaw 的结构来自当前官方资料，本机未发现可验证实例，因此相关结论保留中等置信度；Hermes 外部 Provider 的通用枚举能力没有官方保证，因此不作完整接入承诺。

## Conclusion

Tessera 值得进入产品定义和实现阶段，但第一阶段必须保持克制：它首先是一个可信、可重建、来源透明的本地 Memory Explorer。其竞争力不来自替代 Codex、Claude Code、Hermes 或 OpenClaw 的记忆系统，而来自把这些系统重新置于用户控制之下。

下一步建议使用 `$bmad-create-prd` 将本研究转化为 MVP 功能、非目标和验收标准；随后使用 `$bmad-create-architecture` 固化 Adapter contract、canonical schema、索引 generation、IPC 和安全边界。

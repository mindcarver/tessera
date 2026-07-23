# Tessera Requirements Matrix

本 companion 保存 SPEC 内核之外的可验证需求、术语和来源边界；下游实现与验证必须同时读取它和 adopted architecture spine。

## Functional requirements

| ID | Requirement | Acceptance signal |
| --- | --- | --- |
| FR-1 | 自动发现 Codex/Claude Code Candidate Source | 显示 Provider、候选路径、发现依据、Native Project；不读聊天记录；无结果时无手动目录入口 |
| FR-2 | 确认、拒绝、停用 Source | 未确认来源不进索引；确认状态重启保留；停用不改原始记忆 |
| FR-3 | Source Inventory | 显示 Provider、路径、Native Project、Coverage Level、Source Health、最近成功扫描、记录数、最近错误 |
| FR-4 | 保留 Native Project | 原样保存 Provider 项目标识；无法确认时显示未映射，不猜测 repository |
| FR-5 | 建立 Tessera Project 映射 | 支持跨 Provider 多对一映射；仅写本地状态；可调整/移除且不删源数据 |
| FR-6 | 限定 Agent Memory 边界 | 排除聊天、transcript、CLAUDE.md、AGENTS.md、规则文件；记录标明 Provider Memory 类型 |
| FR-7 | 只读建立 Derived Index | 源文件集合、内容、大小、修改时间不被改变；失败不替换上一成功版本 |
| FR-8 | 更新 Derived Index | 成功扫描反映新增/修改/删除；状态可见；手动扫描只作用于指定 Confirmed Source |
| FR-9 | 搜索 Confirmed Source | 默认搜索健康且成功索引的来源；不调用远程模型；区分无匹配、未索引、不可用 |
| FR-10 | 筛选搜索结果 | 可按 Provider、Source、Tessera Project、Native Project、Memory 类型、时间组合筛选；范围可见 |
| FR-11 | 展示结果与 Provenance | 含 Provider、Source、Native Project、原始文件/引用、定位、更新时间、Coverage Level、Source Health；不冒充推断事实 |
| FR-12 | 打开原始位置 | 用户可从结果定位到原始文件或 Provider 引用；不可用位置明确报错 |
| FR-13 | 展示 Source Health | 显示 unknown/healthy/degraded/error、原因、上次成功时间、Coverage Level 和扫描状态 |
| FR-14 | 隔离 Connector 失败 | 单 Source 失败不阻断其他来源；上一可用索引不被半次失败覆盖 |
| FR-15 | 重建 Derived Index | 可删除并从 Confirmed Source 重建；损坏/缺失时有明确状态和恢复入口 |
| FR-16 | 浏览 Agent Memory 集合 | 可按时间、Native Project、Memory 类型浏览，并打开任意卡片 Provenance |
| FR-17 | 可视化记忆结构 | 展示 Provider、Source、Tessera Project、Native Project、Memory 集合层级；不添加无法验证的语义关系 |
| FR-18 | 本地启动与使用 | 无账号、无联网依赖完成发现、确认、索引、查询、浏览和打开；重启后保留本地确认与映射 |

## Non-functional requirements

| ID | Requirement | Acceptance signal |
| --- | --- | --- |
| NFR-1 | Source 是事实源，Index 是可重建视图 | 删除索引后仍可从 Confirmed Source 重建 |
| NFR-2 | 正常运行不上传本机资产 | 网络访问被本地权限边界阻止或无调用 |
| NFR-3 | 日志不记录敏感正文 | 默认日志不含 Memory 正文、查询词、凭据 |
| NFR-4 | 未来远程 Knowledge Source 显式授权 | 未配置授权时不连接远程来源 |
| NFR-5 | 只读取 Confirmed Source 范围 | UI/IPC 无任意文件读取能力 |
| NFR-6 | 来源边界持续校验 | 路径、symlink、权限、身份变化触发重新校验 |
| NFR-7 | 不可信内容安全渲染 | HTML、脚本、命令不会执行 |
| NFR-8 | 单 Source 故障隔离 | 其他来源仍能搜索和浏览 |
| NFR-9 | 原子可见切换 | 失败扫描保留上一成功 generation |
| NFR-10 | 索引损坏可恢复 | 仅依赖 Confirmed Source 完整重建 |
| NFR-11 | 真实数据性能基准 | Phase 0 生成搜索、扫描、内存、索引体积基准，阈值不凭空设定 |
| NFR-12 | 扫描不阻断旧结果查询 | 扫描期间上一成功索引继续可查询 |
| NFR-13 | 键盘可用 | 发现、搜索、筛选、打开核心路径可用键盘完成 |

## MVP supported artifact matrix

| Provider | Included | Excluded |
| --- | --- | --- |
| Codex | 官方记忆目录中的自动生成 Markdown，如 `MEMORY.md`、`memory_summary.md`、`raw_memories.md`、`rollout_summaries/*.md` | rollout/transcript JSONL、session 内容、状态数据库中的对话内容 |
| Claude Code | 项目 auto-memory 目录中的 `MEMORY.md` 与 topic Markdown | `CLAUDE.md`、`AGENTS.md`、`.claude/rules`、session/transcript |

具体文件集合随 Provider 版本变化；Connector 必须使用 fixtures 验证并记录 parser version，格式变化时显示 degraded。

## Terminology

- **Agent Memory:** Provider 自动生成并持久化的记忆工件，不含原始聊天、人工指令和项目规则。
- **Provider:** 产生 Agent Memory 的 Agent 产品；MVP 为 Codex 与 Claude Code。
- **Connector:** 发现、读取和解释 Provider Agent Memory 的只读连接器。
- **Candidate/Confirmed Source:** 自动发现但未确认 / 已获用户允许读取的来源。
- **Native Project:** Provider 原生项目标识、目录键或 repository 作用域，Tessera 原样保留。
- **Tessera Project:** 本地跨 Provider 项目视图，不修改源数据。
- **Derived Index:** 可删除、可重建的本机索引，不是事实源。
- **Coverage Level:** `full`、`search_only`、`existence_only`、`unsupported`。
- **Source Health:** `unknown`、`healthy`、`degraded`、`error`。
- **Provenance:** 结果的 Provider、Source、Native Project、原始位置、定位、更新时间和内容标识。
- **Knowledge Source:** 未来的 Obsidian、RAGFlow、飞书知识库等独立来源类型。

## Future direction (not MVP)

Obsidian 使用本地只读 Vault Connector；RAGFlow 使用官方 API/MCP 查询并保留 dataset/document/chunk citation；飞书知识库继承显式授权和权限模型。若未来向其他 Agent 暴露查询，顺序为 transport-neutral Query Service、`tessera search --json`、MCP stdio，最后才评估 localhost HTTP；不得暴露写回、删除、任意路径、任意 SQL 或 Provider 凭据。

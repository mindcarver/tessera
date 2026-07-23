---
id: SPEC-tessera
companions:
  - requirements-matrix.md
  - ../../planning-artifacts/architecture/architecture-tessera-2026-07-20/ARCHITECTURE-SPINE.md
sources:
  - ../../planning-artifacts/prds/prd-tessera-2026-07-20/prd.md
  - ../../planning-artifacts/prds/prd-tessera-2026-07-20/addendum.md
  - ../../planning-artifacts/research/technical-codex-claude-code-hermes-openclaw-memory-integration-research-2026-07-20.md
---

# Tessera 本地跨 Agent Memory 联邦规范

## Why

个人开发者在 Codex 与 Claude Code 之间切换时，原生记忆分散在不同来源；工具替换、来源失效或项目切换都会让已有知识变得不可见。Tessera 要在本机以只读方式统一清点、搜索、浏览和追溯这些原生 Agent Memory，同时保留事实源在原位，使 Agent 可以替换而记忆资产仍可定位。

## Capabilities

- **CAP-1**
  - **intent:** 自动发现 Codex 与 Claude Code 的 Candidate Source，让用户知道本机有哪些可接入的 Agent Memory 来源。
  - **success:** 启动后显示 Provider、候选路径、发现依据和可判定 Native Project；发现阶段不读取聊天记录。
- **CAP-2**
  - **intent:** 用户可以确认、拒绝或停用 Source，控制哪些来源进入 Tessera。
  - **success:** 只有 Confirmed Source 扫描正文；确认状态重启后保留；操作不修改原始记忆。
- **CAP-3**
  - **intent:** 用户可以查看所有 Source 的范围、覆盖能力和健康状态。
  - **success:** Inventory 显示 Provider、路径、Native Project、Coverage Level、Source Health、最近成功扫描、记录数与错误；不完整能力不显示为完整同步。
- **CAP-4**
  - **intent:** 用户可以保留 Provider 原生项目身份，并把多个 Provider 的 Native Project 映射到 Tessera Project。
  - **success:** 未确认映射保持未映射；映射只写本地状态；调整或移除映射不删除源数据或索引记录。
- **CAP-5**
  - **intent:** Tessera 只纳入 Provider 自动生成的 Agent Memory，形成可重建的统一记录视图。
  - **success:** 聊天、transcript、人工规则文件被排除；每条记录保留 Provider Memory 类型；原始来源不被修改。
- **CAP-6**
  - **intent:** 用户可以建立、更新、删除后重建 Agent Memory 的 Derived Index。
  - **success:** 新增、修改、删除在成功扫描后可查询；失败扫描不替换上一成功版本；索引删除后可仅依赖 Confirmed Source 重建。
- **CAP-7**
  - **intent:** 用户可以跨 Codex 与 Claude Code 搜索并按来源、项目、类型和时间筛选。
  - **success:** 组合筛选范围可见；空结果、未索引和当前不可用被明确区分；查询不依赖外部模型或远程搜索。
- **CAP-8**
  - **intent:** 用户可以查看原始记忆片段、完整 Provenance，并打开原始位置。
  - **success:** 结果包含 Provider、Source、Native Project、原始文件或引用、定位、更新时间、Coverage Level 与 Source Health；不把推断内容冒充原始事实。
- **CAP-9**
  - **intent:** 用户可以在单个 Connector 失败时继续使用其他 Source，并重新扫描恢复。
  - **success:** 失败 Source 显示原因与上次成功时间；旧索引状态明确；其他 Source 仍可搜索和浏览。
- **CAP-10**
  - **intent:** 用户可以在没有查询词时按时间、Native Project 和 Memory 类型浏览 Agent Memory 集合与结构。
  - **success:** 浏览与搜索共享索引、Provenance 和健康状态，并可打开任意原始位置。
- **CAP-11**
  - **intent:** 用户可以在无账号、无联网依赖的本机本地 Web 应用（本地服务 + 浏览器界面）中完成发现、索引、搜索、浏览和来源打开。
  - **success:** 正常 MVP 运行不上传 Agent Memory、查询词、项目映射或诊断数据；核心操作支持键盘完成。

## Constraints

- Agent Memory 原生文件是唯一事实源；Tessera 和 Derived Index 只读，索引必须可删除、可重建。
- MVP 只支持 Codex 与 Claude Code 的自动发现来源，不提供任意目录手动添加。
- 只扫描用户确认的 Source；路径、符号链接、权限或身份变化必须重新校验。
- 展示内容按不可信文本处理，不执行 HTML、脚本或命令；日志默认不记录正文、查询词和凭据。
- 单 Source 故障必须隔离；扫描只有完整成功才能切换可见 generation，旧成功索引必须保留。
- watcher 只能触发 reconcile，不能直接改变可见索引；搜索和浏览共享同一索引状态。
- Source identity 使用持久 `source_id` 与版本化确定性 fingerprint；歧义不得自动合并，必须显式 rebind。
- generation 使用 source revision、fencing token 和 `snapshot-at-validation`；`dirty_after_validation` generation 不得激活。
- Agent Memory 与未来 Knowledge Source 是不同领域类型；MVP 不实现 Obsidian、RAGFlow 或飞书知识库。
- 技术基线为本地 Web 应用：Rust 核心内嵌 loopback-only HTTP 服务、React/TypeScript/Vite 浏览器 UI、SQLite FTS5；精确版本与构建门禁由 Phase 0 锁定。
- 性能阈值以 Carver 真实数据建立基准，不在基准前编造固定数字；核心发现、搜索、筛选和打开操作需支持键盘。

## Non-goals

- 索引聊天记录、session transcript、人工规则或项目指令文件。
- 自动摘要、语义推理、写回、冲突解决或统一记忆后端。
- Hermes/OpenClaw、Obsidian、RAGFlow、飞书知识库 Connector。
- 云端、多设备、移动端、团队共享、权限协作、MCP/CLI、对外或远程 HTTP 服务（localhost UI 服务是交付机制，不是对外服务面）。

## Success signal

Carver 在一个本机界面中确认“Codex 有什么、Claude Code 有什么”，能够按项目搜索或浏览两者的原始 Agent Memory，并从每条结果追溯到来源；当一个 Source 失效时，仍能区分可用旧结果、不可用结果和确实无匹配，而不会误把静默失败当成没有记忆。

## Assumptions

- 首个真实用户是 Carver，MVP 运行在其本机 macOS 开发环境。
- 受支持记忆格式由 fixtures 和 parser version 管理；格式漂移显示 degraded，而不是静默漏读。

## Open Questions

- Story 1.1 重做应锁定的 HTTP 服务器选型、端口策略与 loopback 安全验收是什么？
- 真实数据基准完成后，搜索延迟、首次扫描、内存和索引体积的可接受阈值是什么？
- 后续版本是否允许一个 Native Project 绑定多个 Tessera Project，还是保持显式唯一归属？

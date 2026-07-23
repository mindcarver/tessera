# Tessera PRD Addendum

本文件保存不应进入产品需求正文、但对后续架构和产品演进有价值的技术与市场背景。产品范围以 [prd.md](./prd.md) 为准。

## 1. 输入材料

- [Forged Idea](../../../forge/user-owned-agent-brain-os/forged-idea.md)
- [技术研究报告](../../research/technical-codex-claude-code-hermes-openclaw-memory-integration-research-2026-07-20.md)

## 2. 当前技术方向

2026-07-22 起 MVP 交付形态为本地 Web 应用（不再使用 Tauri 桌面壳，见 sprint-change-proposal-2026-07-22）。当前基础组合为：

- Rust core 内嵌 loopback-only HTTP 服务作为应用与权限边界（仅绑 127.0.0.1，Host/Origin 校验，CSP 响应头）；
- Rust 负责 Source Registry、Connector、路径防护、扫描协调、Derived Index 和查询；
- React + TypeScript + Vite 负责 Source Inventory、项目映射和搜索界面（系统浏览器承载）；
- SQLite FTS5 保存可删除、可重建的派生全文索引；
- 文件变化事件只触发 reconcile，不能直接作为索引事实。

架构阶段必须重新验证：

- HTTP endpoint 是否全部纳入带 `api_version` 的版本化契约，且仅接受 `source_id`/`record_id`；
- 浏览器 UI 不获得文件系统、shell、任意 SQL 或任意路径能力；
- 源 SQLite 的只读模式不会产生 sidecar；无法证明时优先读取官方文件记忆或降级；
- generation staging 与原子切换保证失败扫描不产生半套可见索引；
- 稳定 record ID 只在输入来源和 parser version 不变的条件下承诺；content hash 用于变化检测，不作为唯一身份；
- watcher 只能触发周期性或受限 reconcile，不能直接改变可见索引；丢事件必须由 reconcile/self-healing 测试覆盖；
- Markdown 和所有 Agent Memory 均按不可信内容安全渲染。

### 2.1 MVP Supported Artifact Matrix

| Provider | MVP 读取 | MVP 排除 | 备注 |
|---|---|---|---|
| Codex | 官方记忆目录中的自动生成 Markdown 记忆工件，如 `MEMORY.md`、`memory_summary.md`、`raw_memories.md`、`rollout_summaries/*.md` | 原始 rollout/transcript JSONL、session 内容、状态数据库中的对话内容 | 具体文件集合随 Codex 版本变化，Connector 必须用 fixture 验证 |
| Claude Code | 项目 auto-memory 目录中的 `MEMORY.md` 与 topic Markdown | `CLAUDE.md`、`AGENTS.md`、`.claude/rules`、session/transcript | project key 无法反向确认真实仓库时保留原始 key |

这是产品边界的实现清单，不代表所有未来版本的永久格式承诺。Connector 需要记录 parser version，并在格式变化时显示 degraded，而不是静默漏读。

## 3. 建议的数据边界

技术实现不应把所有来源内容压成同一种对象。为未来扩展建议保留：

```text
Source
├── source_kind: agent_memory | local_knowledge | remote_knowledge
├── provider
├── capabilities
├── health
└── Collection
    └── Item
        ├── canonical_project
        ├── native_identity
        ├── provenance
        └── open_target
```

MVP 只实现 `agent_memory`。`local_knowledge` 与 `remote_knowledge` 是未来类型，不应为了预留而加入无实际用途的 UI、表单或 Connector。

## 4. 竞品与相邻产品研究摘要

截至 2026-07-20，OpenClaw 已成为最直接的近邻，而不只是参考对象：

- OpenClaw Control UI 可导入 Codex 与 Claude Code 记忆；onboarding 和 migrate 能力还覆盖 Hermes。
- Mem0 提供跨 Agent 共享记忆、语义搜索、自托管 Dashboard 和 MCP。
- Pieces for Developers 提供本地开发者工作记忆、跨工具捕获和 MCP。
- AnythingLLM 提供本地 Workspace、记忆 UI、自动抽取和 RAG。
- Obsidian、RAGFlow、飞书知识库分别覆盖本地 Markdown、异构知识检索和企业知识空间。

因此以下表述不是可靠差异：本地运行、隐私、支持多个 Agent、全文/语义搜索、来源引用、Dashboard 或 MCP。

可守住的定位是：

> Tessera 不要求用户把原生记忆迁移到另一个 Agent 或统一记忆引擎，而是持续、只读地清点和查询各 Agent 原生事实源，并明确展示项目映射、Provenance、Coverage Level、Source Health 与格式漂移。

关键官方来源：

- [OpenClaw Memory](https://docs.openclaw.ai/concepts/memory)
- [OpenClaw Control UI](https://docs.openclaw.ai/web/control-ui)
- [OpenClaw Migrate](https://docs.openclaw.ai/cli/migrate)
- [Mem0 OSS](https://docs.mem0.ai/open-source/overview)
- [Pieces Desktop](https://docs.pieces.app/products/desktop)
- [AnythingLLM Memories](https://docs.anythingllm.com/features/memories)
- [Obsidian Data Storage](https://obsidian.md/help/Files%2Band%2Bfolders/How%2BObsidian%2Bstores%2Bdata)
- [RAGFlow](https://github.com/infiniflow/ragflow)
- [飞书知识库 API](https://open.feishu.cn/document/ukTMukTMukTM/uUDN04SN0QjL1QDN/wiki-v2)

## 5. 后续 Knowledge Source 接入方向

### Obsidian

优先采用本地只读 Vault Connector。Markdown 文件是事实源；Obsidian CLI 可作为增强能力，但不应成为读取 Vault 的硬依赖。

### RAGFlow

优先通过官方 API 或 MCP 提供远程查询 Connector，不复制其完整切块和向量数据库。结果需要保留 dataset、document、chunk citation、权限和最近同步状态。

### 飞书知识库

应使用用户显式授权并继承飞书权限模型。任何本地缓存都要显示最近同步时间、授权身份和清除入口；不得宣传为完全离线来源。

## 6. 未来 Agent 查询接口

若未来需要让其他 Agent 查询 Tessera，推荐顺序是：

1. 先稳定 transport-neutral Query Service；
2. 增加 `tessera search --json`；
3. 再提供 MCP stdio server；
4. 对外共享查询的 localhost HTTP 需另行评估——注意 UI transport 自 2026-07-22 起已是 loopback-only HTTP（仅服务本机浏览器 UI，见 sprint-change-proposal-2026-07-22），不作为对其他 Agent 开放的查询面。

这些接口不属于 MVP，也不能暴露写回、删除、任意路径、任意 SQL 或 Provider 凭据。

# Tessera — Agent 指引

> 本文件是给非 Claude agent 工具（Codex 等）的项目速查。**权威约定以 [`CLAUDE.md`](./CLAUDE.md) 为准**；本文件保持与其一致，若两者冲突以 CLAUDE.md 为准。

## 项目简介

本地优先（local-first）的跨 Agent 记忆联邦浏览层：Rust core 是唯一的文件/SQLite 边界，HTTP 仅绑定 `127.0.0.1`，React UI 只调用版本化 HTTP API。对所有 Provider 源（Codex、Claude Code 等）零写入、不读取聊天/transcript 正文。

## 分支与合并工作流（强制）

- **所有改动通过 feature 分支 + Pull Request 合并到 `main`；禁止直接提交到 `main`。**
- 分支命名：`feat/<story>`、`fix/<topic>`、`chore/<topic>`（例：`feat/story-4-3-path-change-degraded`）。
- PR 合并后删除已合并的 feature 分支。
- 自动化流程（如 BMAD `dev-auto`）同样遵守：产出落在 feature 分支上，通过 PR 合并；**不要**直接 `git commit` 到 `main`，也**不要**直接 `git push origin main`。
- 例外：纯文档或产物同步等无风险改动可酌情直接提交，但仍优先走 PR。

## 开发约定速记

- Rust 包位于 `server/`（包名 `tessera`）；前端在仓库根（Vite + React）。验证用 `cargo test --manifest-path server/Cargo.toml` 与 `npm run build`——**不要**用 `-p tessera-server`（包名不是它，从仓库根会报 "could not find Cargo.toml"）。
- 规划/实现产物在 `_bmad-output/`（`planning-artifacts`、`implementation-artifacts`）。
- 提交信息用 Conventional Commits（`feat:` / `fix:` / `chore:` / `test:` …）。

## Story 进度（Epic 4）

权威状态以 `_bmad-output/implementation-artifacts/sprint-status.yaml` + 各 `spec-<n>-*.md` frontmatter 为准；本表是速查索引，更新可能滞后。详细进度与跨 story 决策见 [`CLAUDE.md`](./CLAUDE.md#story-进度epic-4)。

| Story | 标题 | 状态 | Spec | 实现提交 |
|---|---|---|---|---|
| 4.1 | 文件变化 watcher hint 与 reconcile 自动刷新 | done | `spec-4-1-watcher-reconcile.md` | `9cdbc3d`（PR #9 合并） |
| 4.2 | Connector 失败隔离与 stale 上一成功结果 | done | `spec-4-2-connector-failure-isolation-stale-last-success.md` | `99dfb25`（PR #9 合并到 main：`f3fb14c`） |
| 4.3 | 路径/权限/身份变化的重发现与 degraded 处理 | done | `spec-4-3-path-change-degraded.md` | `a1eb858`（feat）+ `ba6744c`（chore spec 回填） |
| 4.4 | Derived Index 整体重建 | backlog | — | — |

## Agent 工作守则（Epic 4 沉淀）

改动 Epic 4 代码前必读：

- **Watcher-as-hint, reconcile-as-truth（AD-8）**：`notify` 事件只是 dirty hint；真相来自受限 reconcile。watcher 事件**绝不**直接增删 canonical records。
- **失败是 source-scoped（AD-13）**：单个 Connector 失败不阻塞其他 Source。失败 Source 的上一成功 generation 必须保留可查，标 degraded + cause + last-success + stale。
- **`HealthCause` 单一写入入口**：所有 health + cause 通过 `SourceRegistry::set_health_and_cause` 原子写入，勿分散更新。
- **Rebind 语义（AD-33/AD-35）**：root 变化时旧 Source 标 `Disabled`（保留 cause + last-success），新 Source 在新 fingerprint 上 Confirmed。`native_project` 从新 root 重新派生（不复制旧行）；disable + insert 必须在单个 SQLite 事务内。
- **零源变异**：扫描/reconcile/rebuild 前后源文件集合/内容/size/mtime 不变。
- **脱敏（NFR-3）**：错误消息/日志不含正文、查询文本、凭据。

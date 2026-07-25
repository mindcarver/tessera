# Tessera

本地优先（local-first）的跨 Agent 记忆联邦浏览层：Rust core 是唯一的文件/SQLite 边界，HTTP 仅绑定 `127.0.0.1`，React UI 只调用版本化 HTTP API。对所有 Provider 源（Codex、Claude Code 等）零写入、不读取聊天/transcript 正文。

## 分支与合并工作流（强制）

- **所有改动通过 feature 分支 + Pull Request 合并到 `main`；禁止直接提交到 `main`。**
- 分支命名：`feat/<story>`、`fix/<topic>`、`chore/<topic>`（例：`feat/story-2-1-claude-discover`）。
- PR 合并后删除已合并的 feature 分支。
- 自动化流程（如 BMAD `dev-auto`）同样遵守：产出落在 feature 分支上，通过 PR 合并；**不要**直接 `git commit` 到 `main`，也**不要**直接 `git push origin main`。
- 例外：纯文档或产物同步等无风险改动可酌情直接提交，但仍优先走 PR。

## 开发约定速记

- Rust 包位于 `server/`（包名 `tessera`）；前端在仓库根（Vite + React）。验证用 `cargo test --manifest-path server/Cargo.toml` 与 `npm run build`——**不要**用 `-p tessera-server`（包名不是它，从仓库根会报 "could not find Cargo.toml"）。
- 规划/实现产物在 `_bmad-output/`（`planning-artifacts`、`implementation-artifacts`）。
- 提交信息用 Conventional Commits（`feat:` / `fix:` / `chore:` / `test:` …）。

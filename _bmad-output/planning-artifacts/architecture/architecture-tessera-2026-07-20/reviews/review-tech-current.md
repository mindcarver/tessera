# 技术时效性审查（AD-35/AD-36 后最终复核）

审查对象：最新 `ARCHITECTURE-SPINE.md` 的 Stack seeds、Phase 0 exact build/SQLite checks，以及 AD-35/AD-36。

审查日期：2026-07-20

## Verdict

**PASS**。AD-35 的版本化确定性 source fingerprint 与 AD-36 的 `snapshot-at-validation` fence/retry 规则是应用层一致性约束，不引入新的第三方技术或版本风险；两者与 Tauri/Rust/SQLite/notify 选择相容。Stack seeds 仍为当前可用版本线，exact build 和 SQLite/FTS5 验证也保持在正确的 Phase 0 边界。

## Version and fit verification

| Technology | Spine seed | Verification and fit |
| --- | --- | --- |
| Tauri | 2.x | Official releases are in the 2.11.x line, including 2.11.3: <https://github.com/tauri-apps/tauri/releases>. Fits capability-controlled WebView plus Rust core. Exact CLI/runtime/bundler remains Phase 0. |
| Rust | stable 1.97.x | Official notes list 1.97.1 (2026-07-16): <https://doc.rust-lang.org/stable/releases.html>. Fits the single local process; exact patch is correctly deferred to `rust-toolchain.toml`. |
| React / React DOM | 19.2.7 | Official versions page lists 19.2.7: <https://react.dev/versions>. Fits local inventory/search/browse UI. |
| Vite | 8.1.x | Official support page identifies 8.1 as the regular-patch line: <https://vite.dev/releases>. Fits Tauri frontend builds. |
| rusqlite | 0.40.1 + `bundled` | Current stable docs list 0.40.1 and bundled SQLite support: <https://docs.rs/crate/rusqlite/latest>. Fits deterministic local derived indexing. |
| SQLite | bundled 3.x + FTS5 | Official download page lists 3.53.3 and FTS5 docs confirm the feature: <https://www.sqlite.org/download.html>, <https://www.sqlite.org/fts5.html>. Fits MVP full-text search. |
| notify | 8.2.x | Docs.rs lists 8.2.0 stable; 9.0 remains RC: <https://docs.rs/crate/notify/latest>. Fits dirty hints; AD-34 correctly prevents post-validation mutations from becoming active. |

## Top findings

1. **PASS — AD-35/AD-36 are compatible with the selected stack.** They use deterministic filesystem metadata and transactional fencing, all implementable within Rust + bundled SQLite without new infrastructure.
2. **Phase 0 exact build gate remains required.** Resolve and record exact Tauri CLI/runtime/bundler, Rust patch, frontend lockfile, rusqlite/libsqlite3-sys, bundled SQLite and notify patch before enabling adapters.
3. **SQLite/FTS5 check must be executable.** Verify `sqlite_version()`, FTS5 virtual-table creation/query, and record the result with `tests/benchmarks/memory-index.json`; this remains correctly Deferred until Phase 0.
4. **Tauri 2.x is intentionally a family seed.** Exact cross-package compatibility must come from the real bootstrap build, not from the major number.

## Confirmed boundaries

- React 19.2.7, Vite 8.1.x, rusqlite 0.40.1, notify 8.2.x, Rust 1.97.x and SQLite FTS5 are current and suitable.
- Exact Rust/Tauri patch, macOS minimum, signing, packaging, updater and remote runtime remain correctly Deferred.
- No HTTP server, vector store, cloud SDK or remote connector is needed at this architecture stage.


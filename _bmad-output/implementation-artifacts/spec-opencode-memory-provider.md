---
title: 'OpenCode persistent-instruction memory provider'
type: 'feature'
created: '2026-07-26'
status: 'done'
baseline_commit: 'e274f0df3b9f6f54169bbfdfe0ce3ef2d083db1a'
review_loop_iteration: 0
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-2-context.md'
  - '{project-root}/docs/phase-0-verification.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Tessera cannot discover or search durable OpenCode context. OpenCode 1.17.7 has no first-party memory table or memory directory: its durable first-party context is instruction files, while its SQLite database mainly stores project and session data.

**Approach:** Add an `opencode` provider that treats OpenCode-owned `AGENTS.md` files as read-only memory artifacts. Discover the global instruction file from OpenCode's config directory and project-root instruction files from read-only `project` metadata in `opencode.db`; reuse Tessera's Markdown canonicalizer and existing scan/search/inventory pipeline.

## Boundaries & Constraints

**Always:**
- Use provider id `opencode`, parser tag `opencode-agents-md/v1`, `Full` coverage for each exact one-file Source, and memory type `agent_instruction`.
- Respect absolute `OPENCODE_CONFIG_DIR`, `XDG_CONFIG_HOME`, and `XDG_DATA_HOME` paths, with HOME-based defaults `~/.config/opencode` and `~/.local/share/opencode`.
- Open `opencode.db` read-only and query only `project.id` plus `project.worktree`; project candidates require an existing absolute worktree and direct-child `AGENTS.md`.
- Global candidate: config directory containing direct-child `AGENTS.md`, `native_project = null`. Project candidate: worktree root, `native_project = OpenCode project.id`.
- Enumerate only the defining direct-child `AGENTS.md`; unrelated repository/config files are ignored, symlink escape is rejected, and scanning leaves source files byte/size/mtime-identical.
- OpenCode roots use non-recursive watcher registration. Rebind must re-derive global/project identity from current OpenCode metadata and fail closed before disabling the old Source when identity is ambiguous.
- Discovery/database failures safe-degrade per Source and never block Codex or Claude Code.

**Ask First:** Supporting third-party memory plugins, custom `instructions` globs/URLs, nested instruction files, OpenCode's `CLAUDE.md` compatibility fallback, or any session-derived source.

**Never:** Invent `~/.opencode/memories`; read `auth.json`, logs, credentials, or any `session`, `message`, `part`, `session_input`, `session_message`, or prompt/body field; write to OpenCode files/SQLite; recursively crawl project worktrees.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Global instructions | config dir has `AGENTS.md` | One `opencode_global_config` candidate, global native scope | No error |
| Project instructions | project table has two absolute worktrees; one has root `AGENTS.md` | One `opencode_project_database` candidate with that project id | Missing file/worktree is skipped |
| Database unavailable | missing, locked, malformed, or incompatible `opencode.db` | Global discovery still works; no project candidates | No panic or content fallback |
| Scan project source | root also contains `.git`, source, and build output | Only `AGENTS.md` is indexed as `agent_instruction` | Unrelated entries ignored |
| Defining file changes | confirmed Source loses or corrupts `AGENTS.md` | Previous successful generation remains queryable and Source degrades | Source-scoped safe error |

</frozen-after-approval>

## Code Map

- `server/src/adapters/opencode.rs` -- read-only config/project discovery and exact-file enumeration.
- `server/src/domain/ports/provider_adapter.rs` -- OpenCode discovery bases and `agent_instruction` vocabulary.
- `server/src/application/source.rs` -- adapter registration, discovery aggregation, and rebind identity.
- `server/src/application/reconcile.rs`, `server/src/http/mod.rs` -- provider-aware non-recursive watch lifecycle.
- `server/src/domain/{query,project}.rs` -- provider/filter/mapping allowlists.
- `src/api/{discover,search}.ts`, `src/components/providerDisplayName.ts` -- wire mirrors and OpenCode UI label.
- `server/tests/opencode_{discover,scan}.rs`, `server/tests/{source_registry,search,http_api,reconcile}.rs`, `tests/ui/accessibility.spec.ts` -- contract and UI coverage.
- `docs/phase-0-verification.md` -- document the read-only project-metadata SQLite exception.

## Tasks & Acceptance

**Execution:**
- [x] Implement `OpenCodeAdapter` discovery/enumeration with injected-path test seams and no session-content access.
- [x] Register `opencode`; synchronize Rust/TypeScript discovery, memory-type, query, and project vocabularies.
- [x] Make watcher depth and rebind identity provider-aware without changing Codex/Claude behavior.
- [x] Add fixtures and tests for the matrix, zero mutation, parser version, provider filtering, inventory/search visibility, and read-only database behavior.
- [x] Update the Phase 0 SQLite boundary note.

**Acceptance Criteria:**
- Given OpenCode project metadata and a project-root `AGENTS.md`, when discovery, confirmation, and scan run, then searchable `opencode` records appear with project id, provenance, `agent_instruction`, and `opencode-agents-md/v1`.
- Given a database containing only the `project` table, when discovery runs, then it succeeds, proving no session/content table dependency.
- Given OpenCode plus healthy Codex/Claude Sources, when OpenCode discovery or scan fails, then the other providers remain usable and the last successful OpenCode generation is preserved.
- Given the UI, when Sources/Search render, then OpenCode is visible and provider/type filters accept `opencode`/`agent_instruction`.

## Spec Change Log

- 2026-07-26: Implemented the approved OpenCode provider boundary and completed automated verification.
- 2026-07-26: Review fixes made SQLite row decoding all-or-nothing, rejected ambiguous roots and invalid project ids, and added XDG, locked/read-only DB, rebind, watcher, wire, and project-mapping regression coverage.

## Design Notes

OpenCode's instruction resolver can also load compatibility files, nested rules, configured globs, and URLs. Those surfaces are excluded because they are not a bounded first-party memory store and would require session context or remote reads. The Source boundary is deliberately one direct-child `AGENTS.md` per root.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- passed: 490 tests, 1 ignored probe, 0 failed.
- `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings` -- passed with no warnings.
- `npm run build` -- passed: TypeScript/Vite production build.
- `npx playwright test tests/ui/accessibility.spec.ts` -- passed: 19 tests.
- `git diff --check` -- passed.

**Manual checks (if no CLI):**
- Compare file set, content, size, and mtime for a real OpenCode `AGENTS.md` Source before and after scan; verify no session/message content is exposed.

## Suggested Review Order

**Provider boundary**

- Start with the complete first-party discovery and exact-file enumeration contract.
  [`opencode.rs:29`](../../server/src/adapters/opencode.rs#L29)

- Review the only external SQLite query and all-or-nothing row handling.
  [`opencode.rs:260`](../../server/src/adapters/opencode.rs#L260)

- Confirm instruction memory and discovery wire vocabularies remain explicit.
  [`provider_adapter.rs:67`](../../server/src/domain/ports/provider_adapter.rs#L67)

**Lifecycle and isolation**

- Follow metadata-backed identity resolution before transactional rebind mutation.
  [`source.rs:307`](../../server/src/application/source.rs#L307)

- Verify OpenCode watches and event hints remain strictly non-recursive.
  [`reconcile.rs:497`](../../server/src/application/reconcile.rs#L497)

- Check mapping and search allowlists admit global and project OpenCode scopes.
  [`project.rs:56`](../../server/src/domain/project.rs#L56)

**UI and verification**

- Review global OpenCode mapping-option construction.
  [`Projects.tsx:83`](../../src/features/projects/Projects.tsx#L83)

- Inspect discovery, filter, and null-scope mapping browser coverage.
  [`accessibility.spec.ts:293`](../../tests/ui/accessibility.spec.ts#L293)

- Validate malformed, locked, and read-only database isolation fixtures.
  [`opencode_discover.rs:143`](../../server/tests/opencode_discover.rs#L143)

- Confirm scan provenance, searchability, and zero source-file mutation.
  [`opencode_scan.rs:89`](../../server/tests/opencode_scan.rs#L89)

- Read the documented external SQLite privacy boundary last.
  [`phase-0-verification.md:59`](../../docs/phase-0-verification.md#L59)

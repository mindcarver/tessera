---
title: 'Story 2.1: Claude Code Candidate Source discovery & confirmation'
type: 'feature'
created: '2026-07-25'
status: 'done'
baseline_revision: '0e90d76'
review_loop_iteration: 0
followup_review_recommended: true
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-2-context.md'
warnings: ['oversized']
---

<intent-contract>

## Intent

**Problem:** Tessera federates only Codex today. Cross-agent federation requires Claude Code as a second provider through the same adapter contract — but discovery is hard-wired to Codex (`discover_sources` calls only `CodexAdapter.discover()`) and provider dispatch (`adapter_for`) is mono-morphized to `CodexAdapter`, so no second provider can be discovered or confirmed.

**Approach:** Add a `ClaudeCodeAdapter` that discovers Claude Code auto-memory roots and routes `claude_code` through the existing canonicalize → fingerprint → upsert confirmation pipeline by generalizing dispatch from mono-Codex to a multi-provider adapter registry. Discovery covers two inputs named by the story AC: the official `$CLAUDE_CONFIG_DIR/projects/<project>/memory/` layout (default `~/.claude`), one candidate per project; AND a user-configured `autoMemoryDirectory` read from the user-scope `settings.json`. Generalize Codex-only UI copy and extend the `DiscoveryBasis` wire type so Claude candidates surface honestly alongside Codex.

## Boundaries & Constraints

**Always:**
- One `CandidateSource` per existing `<config_dir>/projects/<project>/memory/` dir. `root_path` = that memory dir; `native_project` = the `<project>` dir name verbatim (no reverse-mapping); `provider = "claude_code"`; coverage from the adapter (single source of truth).
- `CLAUDE_CONFIG_DIR` (absolute, non-empty after trim) overrides `~/.claude`; an explicit-but-invalid (relative) value yields no candidate with **no silent fallback** — mirrors Codex's `CODEX_HOME` rule. Default: `$HOME/.claude`.
- **`autoMemoryDirectory`** (user scope): read `<config_dir>/settings.json` (i.e. `$CLAUDE_CONFIG_DIR/settings.json`, or `~/.claude/settings.json` by default). If it has an `autoMemoryDirectory` key whose value is an absolute path or starts with `~/`, expand a leading `~/` via `HOME` and, if it resolves to an existing UTF-8 directory, emit it as an additional Claude candidate with `basis = ClaudeAutoMemoryDir`, `native_project = None`, `coverage = Full`. Deduplicate against the `projects/*` candidates by canonicalized path (one candidate per physical dir). `serde_json` is already a dependency.
- Discovery is existence/type only (`is_dir`, UTF-8) — never reads memory file contents (NFR-5); zero source mutation (NFR-1). Reading `settings.json` is allowed (it is config, not memory content).
- Confirmation reuses the exact Codex pipeline: `policy::canonicalize_root` → `build_fingerprint` (`root-fingerprint/v1`) → idempotent upsert by fingerprint → `src_<n>`. Claude and Codex Sources coexist as separate `source_registry` rows; re-confirm is an idempotent wake-up returning the same `source_id`.
- Capability honesty (AD-3): the adapter declares only what Claude Code's official memory surface supports.
- **Rescan safety is uniform across surfaces:** a `claude_code` source that is not yet scannable (parsing lands in 2.2) must surface a provider-aware safe message everywhere — the synchronous `/api/scan` envelope, the rescan SSE terminal event, the inventory `latest_error`, and the persisted `scan_runs.error_code` — never a generic `internal` failure. The Codex parser is never applied to Claude files.

**Never:**
- Parsing/indexing Claude memory content (Story 2.2). `enumerate_artifacts`/`enumerate_file_units` for `claude_code` must **hard-fail** (`Err`), not return an empty `Ok` — an empty Ok can activate a false-positive empty generation if the scan guard is ever bypassed.
- Reverse-mapping the encoded `<project>` key to a real repo path (not a stable protocol; Epic 5). Preserve the key verbatim; show as unmapped.
- Reading project-scope `.claude/settings.json` — Tessera has no project context at discovery time. Only the user-scope `<config_dir>/settings.json` is read for `autoMemoryDirectory`.
- Renaming existing `DiscoveryBasis` wire strings (`default_home`, `codex_home_env`) — that breaks the `api_version=1` contract; add new variants alongside.
- Honoring a relative or otherwise invalid `autoMemoryDirectory` value; silently inventing a config contract beyond the documented key.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Default home, multiple projects | `CLAUDE_CONFIG_DIR` unset, `HOME=/h`, `~/.claude/projects/{A,B}/memory/` exist (A empty) | 2 candidates: `provider=claude_code`, roots `…/A/memory` & `…/B/memory`, `native_project=A/B`, `basis=claude_default_home`, `coverage=Full`, sorted by root_path | No error |
| `CLAUDE_CONFIG_DIR` override | `CLAUDE_CONFIG_DIR=/c`, `/c/projects/X/memory/` exists | 1 project candidate, `basis=claude_config_dir_env`; `~/.claude` NOT also scanned | No error |
| Project without `memory/` | `~/.claude/projects/C/` exists, no `memory/` child | No candidate for C (silently skipped) | No error |
| Relative `CLAUDE_CONFIG_DIR` | `CLAUDE_CONFIG_DIR=rel/path` | No candidates; no fallback to `~/.claude` (explicit override final) | Empty result |
| `autoMemoryDirectory` set (absolute) | `settings.json` has `"autoMemoryDirectory": "/abs/m"`, `/abs/m` is a dir | Extra candidate: `basis=claude_auto_memory_dir`, `native_project=None`, root `/abs/m`; coexists with project candidates | No error |
| `autoMemoryDirectory` set (`~/`) | `"autoMemoryDirectory": "~/m"`, `HOME=/h`, `/h/m` is a dir | Extra candidate rooted at `/h/m` (`~/` expanded via HOME) | No error |
| `autoMemoryDirectory` invalid/missing dir | value is relative, or points at a non-existent/non-dir path | No extra candidate; project candidates unaffected (safe degrade) | No error, no panic |
| `settings.json` missing or unparseable | no `settings.json`, or malformed/JSONC-with-comments that fails parse | No `autoMemoryDirectory` candidate; project candidates unaffected (safe degrade) | No error |
| `autoMemoryDirectory` duplicates a project dir | resolves to the same canonical dir as `projects/X/memory/` | Deduped: that dir emits exactly one candidate | No error |
| Confirm Claude candidate | POST confirm `{provider:"claude_code", root_path:…/memory, native_project:X}` | 200, `Source{source_id:"src_<n>", provider:"claude_code", lifecycle:"confirmed", coverage:"full", native_project:"X"}` | No error |
| Re-confirm (idempotent) | same candidate confirmed again | Same `source_id`, lifecycle flipped to `confirmed` | No error |
| Unknown provider | POST confirm `{provider:"unknown"}` | `ConfirmFailed` → safe envelope code `confirm_failed` | Structured error |
| Rescan Claude source | Confirmed `claude_code` source, rescan triggered | Codex parser **not** applied; EVERY surface (sync envelope, SSE terminal event, inventory `latest_error`, `scan_runs.error_code`) shows the provider-aware safe message; not `internal` | Structured error |

</intent-contract>

## Code Map

- `server/src/adapters/claude_code.rs` — `ClaudeCodeAdapter` (`provider_id`, `coverage_level`, `discover` + `discover_with_env` + pure resolver for `projects/*/memory/` + `autoMemoryDirectory` from `settings.json`). `enumerate_*` hard-fails for `claude_code`.
- `server/src/adapters/codex.rs` — reference exemplar for the resolver/discover/candidate pattern; no change expected.
- `server/src/application/source.rs` — `adapter_for` (multi-provider `Box<dyn ProviderAdapter>` registry) and `discover_sources` (union all adapters). Confirm/reject unchanged except dispatch.
- `server/src/domain/ports/provider_adapter.rs` — `DiscoveryBasis` variants incl. `ClaudeDefaultHome`, `ClaudeConfigDirEnv`, `ClaudeAutoMemoryDir` (snake_case serde renames); `CandidateSource` unchanged.
- `server/src/application/scan.rs` / `server/src/domain/scan.rs` / `server/src/http/{mod,envelope}.rs` — `ProviderNotScannable` gets a dedicated `error_code` + `safe_error_reason` arm + a rescan-worker branch so all surfaces show the provider-aware message; the scan guard still refuses `claude_code` until 2.2.
- `server/tests/claude_code_discover.rs` (new) — discover I/O matrix (projects + `autoMemoryDirectory` + dedup + degrade) + capability-honesty + `enumerate` hard-fail.
- `server/tests/source_registry.rs`, `server/tests/http_api.rs` — confirm / zero-source-mutation / unknown-provider / rescan-message-consistency for `claude_code`.
- `src/api/discover.ts` — `DiscoveryBasis` union extended in lockstep with the Rust enum.
- `src/features/sources/Sources.tsx`, `tests/ui/accessibility.spec.ts` — provider-agnostic copy + an assertion that pins it.
- `server/tests/fixtures/providers/claude_code/` — optional seed; tempdir-driven discovery tests suffice (fixture-contract is 2.2).

## Tasks & Acceptance

**Execution:**
- `server/src/domain/ports/provider_adapter.rs` -- add `ClaudeDefaultHome`, `ClaudeConfigDirEnv`, and `ClaudeAutoMemoryDir` to `DiscoveryBasis` with stable snake_case serde renames -- wire-contract extension carrying Claude basis evidence.
- `server/src/adapters/claude_code.rs` -- implement `ClaudeCodeAdapter` (unit struct, `provider_id="claude_code"`, `coverage_level=Full`): `discover`→`discover_with_env(claude_config_dir, home)`→pure resolver→emit one candidate per existing `projects/<project>/memory/` dir (`native_project`=project key, sorted by `root_path`); AND read `<config_dir>/settings.json`, extract `autoMemoryDirectory` (absolute or `~/`-prefixed), expand `~/` via HOME, emit an extra candidate (`basis=ClaudeAutoMemoryDir`, `native_project=None`) when it resolves to an existing dir, dedup by canonicalized path vs project candidates, and degrade safely (no candidate, no error) when `settings.json` is absent/unparseable or the value is invalid -- mirrors `codex.rs`; uses existing `serde_json` dep.
- `server/src/adapters/claude_code.rs` -- make `enumerate_artifacts`/`enumerate_file_units` return `Err` for `claude_code` (not empty `Ok`) so a misrouted scan fails loudly instead of activating a false-positive empty generation.
- `server/src/application/source.rs` -- keep the multi-provider `adapter_for` registry (`Option<Box<dyn ProviderAdapter>>`) so confirm/reject dispatch `claude_code`, and `discover_sources` unions every registered adapter's `discover()` -- second-provider wiring without changing the confirm pipeline or wire contract; `adapter_for_returns_codex_for_known_provider` asserts `claude_code` too.
- `server/src/application/scan.rs` / `server/src/domain/scan.rs` / `server/src/http/envelope.rs` / `server/src/http/mod.rs` -- give `ScanError::ProviderNotScannable` a dedicated stable `error_code` (e.g. `provider_not_scannable`) and a matching `safe_error_reason` arm, and branch on it in the rescan worker so the rescan SSE terminal event (not just the sync `/api/scan` envelope) and the inventory `latest_error` carry the provider-aware message; keep the deny-by-default guard that refuses `claude_code` until 2.2.
- `src/api/discover.ts` -- extend the `DiscoveryBasis` union and `VALID_DISCOVERY_BASES` with the three new variants -- keep TS and Rust wire types in lockstep.
- `src/features/sources/Sources.tsx` / `tests/ui/accessibility.spec.ts` -- keep provider-agnostic copy and add a Playwright assertion that pins the empty-state/loading copy (contains "Agent Memory", not "Codex").
- `server/tests/claude_code_discover.rs` -- cover the projects discover matrix + `autoMemoryDirectory` rows (absolute, `~/`, invalid, missing/unparseable settings, dedup) + `claude_code` capability-honesty + `enumerate` hard-fail -- mirror `codex_discover.rs`'s no-env-mutation tempdir seam.
- `server/tests/source_registry.rs` / `server/tests/http_api.rs` -- confirm / zero-source-mutation / unknown-provider for `claude_code`; strengthen the rescan test to assert the provider-aware message text (not `!is_empty()`) on the SSE terminal event.

**Acceptance Criteria:**
- Given `~/.claude/projects/<P>/memory/` exists, when Carver opens Sources, then a Claude Code candidate appears with `provider=claude_code`, the memory dir as root, `<P>` as Native Project (shown unmapped), and Full coverage.
- Given `CLAUDE_CONFIG_DIR` is set to an absolute dir, when discovery runs, then only `$CLAUDE_CONFIG_DIR/projects/*/memory/` (and `$CLAUDE_CONFIG_DIR/settings.json`) is consulted (not `~/.claude`); given it is relative, then no Claude candidates surface.
- Given `<config_dir>/settings.json` has a valid `autoMemoryDirectory` (absolute or `~/`-prefixed, pointing at an existing dir), when discovery runs, then an extra Claude candidate with `basis=claude_auto_memory_dir`, `native_project=None` appears alongside the project candidates; given the value is invalid, the dir is missing, or `settings.json` is absent/unparseable, then discovery degrades safely (no extra candidate, no error) and project candidates are unaffected.
- Given `autoMemoryDirectory` canonicalizes to the same dir as a `projects/<P>/memory/` candidate, when discovery runs, then that physical dir emits exactly one candidate.
- Given a Claude candidate, when Carver confirms it, then a Source with `source_id=src_<n>`, `provider=claude_code`, `lifecycle=confirmed` is persisted and coexists with Codex sources; re-confirming returns the same `source_id`.
- Given discovery and confirmation run against real Claude dirs and a real `settings.json`, then no memory file's content/size/mtime changes (NFR-1 zero-mutation), and discovery never reads memory file bodies (NFR-5).
- Given a confirmed `claude_code` source, when a rescan is triggered, then the Codex parser is never applied to Claude files and every surface (sync `/api/scan` envelope, rescan SSE terminal event, inventory `latest_error`, persisted `scan_runs.error_code`) shows the provider-aware safe message — never a generic `internal` code.
- Given Codex is the only other provider, when discovery/confirm run, then all existing Codex behavior and the `api_version=1` wire contract are unchanged (no `DiscoveryBasis` rename, no envelope-shape change).
- Given the Sources UI, then candidate/inventory rendering and copy are provider-agnostic (no Codex-only wording, asserted by a test), and discover/confirm/filter remain keyboard-reachable (accessibility contract holds).

## Spec Change Log

### 2026-07-25 — `autoMemoryDirectory` scope correction + review-driven hardening (loopback after intent-gap HALT)
- **Triggering finding:** intent_gap from the first review pass — the verbatim Story 2.1 AC lists a user `autoMemoryDirectory` as a discovery input; an earlier draft of this spec had deferred it on the disproven premise that its mechanism was unverified. Official docs (https://code.claude.com/docs/en/memory, "Storage location") confirm `autoMemoryDirectory` is a real `settings.json` key (absolute path or `~/`-prefixed; any settings scope). Human decision: honor it in 2.1, reading the **user scope** (`~/.claude/settings.json` / `$CLAUDE_CONFIG_DIR/settings.json`).
- **Amended:** moved `autoMemoryDirectory` from deferred to in-scope (Always + I/O matrix + Tasks + AC); removed the old "Block If" and "Never: defer autoMemoryDirectory" clauses. Also folded in review patch findings so re-derivation does not recreate them: (a) `ProviderNotScannable` must carry a dedicated `error_code` and provider-aware message on ALL surfaces, not just the sync envelope; (b) `enumerate_*` for `claude_code` must hard-fail, not return empty `Ok`; (c) the UI-copy and TS-basis changes need asserting tests; (d) the spec's verification commands must use `-p tessera` (package name), not `-p tessera-server`.
- **Known-bad state avoided:** shipping 2.1 without an AC-mandated discovery input; a factually-wrong deferral rationale; a Claude rescan showing a misleading generic `internal` message on the SSE/inventory surfaces; a defensive enumeration fallback that is not actually safe.
- **KEEP (must survive re-derivation):** verified `projects/*/memory/` discovery; `CLAUDE_CONFIG_DIR` priority with no-fallback; multi-candidate-per-project with the project key as `native_project`; reuse of the canonicalize→fingerprint→upsert pipeline; coexistence of Claude and Codex rows; the `Box<dyn ProviderAdapter>` dispatch widening; the deny-by-default `ProviderNotScannable` scan guard that keeps the Codex parser off Claude files until 2.2.

## Review Triage Log

### 2026-07-25 — Review pass
- intent_gap: 1: (high 1)
- bad_spec: 0
- patch: 16: (high 0, medium 6, low 10)
- defer: 3: (low 3)
- reject: 1: (low 1)
- addressed_findings:
  - none

Notes for the next run (patches were mooted by the intent_gap and are NOT applied this pass; the full attempted change is saved at `_bmad-output/implementation-artifacts/story-2-1-attempted-change.patch`):
  - `[high]` `[intent_gap]` The verbatim Story 2.1 AC lists a user `autoMemoryDirectory` as a discovery input; this spec's `<intent-contract>` deferred it on the premise its mechanism was unverified. Official Claude Code docs (https://code.claude.com/docs/en/memory, "Storage location") confirm `autoMemoryDirectory` IS a real `settings.json` key (value: absolute path or `~/`-prefixed; readable from any settings scope). Resolving requires editing the intent-contract, so this HALTs for a human scope decision: honor `autoMemoryDirectory` in 2.1, or confirm deferring to 2.2. — **RESOLVED by human 2026-07-25: honor it, user scope `~/.claude/settings.json`.** Folded into the spec via the Spec Change Log entry above and the re-derivation below.
  - `[medium]` `[patch]` `ProviderNotScannable` cluster (scan.rs / domain/scan.rs / http/mod.rs): `error_code()` maps it to `"internal"`, so the reserved-rescan run row, the inventory `latest_error`, and the rescan SSE terminal event all show a generic/wrong message for a Claude source (only the synchronous `/api/scan` envelope is provider-aware). Give it a dedicated `error_code` + `safe_error_reason` arm + branch in the rescan worker; the existing wire test must assert the provider-aware text, not `!is_empty()`. — **folded into Tasks + AC for re-derivation.**
  - `[medium]` `[patch]` `claude_code.rs` `enumerate_artifacts` returns `Ok(empty)` instead of `Err` — a false-positive empty-generation risk if the scan guard is ever bypassed; hard-fail. — **folded into Tasks + Never + AC.**
  - `[medium]` `[patch]` UI copy generalization (`Sources.tsx`) and the TS `VALID_DISCOVERY_BASES` extension (`discover.ts`) have no asserting test; add coverage. — **folded into Tasks + AC.**
  - `[low]` `[patch]` Spec `## Verification` commands use `-p tessera-server`, but the package is named `tessera`; correct to `-p tessera` (or drop `-p`). — **folded into Verification.**

### 2026-07-25 — Review pass (pass 2: after `autoMemoryDirectory` re-derivation + folded fixes)
- intent_gap: 0
- bad_spec: 0
- patch: 9: (high 0, medium 3, low 6)
- defer: 3: (low 3)
- reject: 4: (low 4)
- addressed_findings:
  - `[medium]` `[patch]` `~/`-expansion now rejects a relative `HOME` and strips extra leading slashes from `~//foo`, so no candidate ever carries a relative/non-HOME-anchored root (`claude_code.rs` + 2 new tests).
  - `[medium]` `[patch]` Codex provider id is now a single source of truth (`CodexAdapter::PROVIDER_ID`, used by both `adapter_for` and the `CODEX_PROVIDER_ID` scan guard) with a sync test — a rename can no longer desync the guard from the registry.
  - `[medium]` `[patch]` Added a Playwright test that mocks `/api/sources/discover` with all three `claude_*` bases and asserts they render — a dropped `VALID_DISCOVERY_BASES` entry now fails loudly (implementer verified by dropping it).
  - `[low]` `[patch]` Provider-aware scan message hoisted to one `pub(crate) const` referenced from the sync envelope, the rescan-SSE worker, and `safe_error_reason`.
  - `[low]` `[patch]` JSONC line-comment scanner now tracks backslash-escaping (doc corrected); SSE poll budget raised to ~5 s; disable-for-Claude test added; spec verification commands corrected to `--manifest-path server/Cargo.toml` (runnable from repo root).
- deferred (see `_bmad-output/implementation-artifacts/deferred-work.md`): `enumerate_*` hard-fail returns `Unreadable` (diagnostic-only — the loud-fail holds; revisit the diagnostic when Claude parsing lands in 2.2); `fail_run`'s `let _ =` on the reserved path (matches existing style; narrow `stale_recovered` override risk); sync `/api/scan` vs rescan persistence asymmetry (inventory `latest_error` only reflects rescan).
- rejected (noise/already-covered): dedup TOCTOU (cosmetic; idempotent confirm collapses it); confirm-with-`claude_auto_memory_dir`-basis test (confirm is basis-agnostic by construction); reject-idempotency for Claude (provider-neutral, covered via Codex + NFR-1); HTTP sync `/api/scan` integration test (mapping pinned by the `map_scan_error` unit test).

## Design Notes

- **`autoMemoryDirectory` is honored at the user scope only.** Read `<config_dir>/settings.json` (`$CLAUDE_CONFIG_DIR/settings.json`, else `~/.claude/settings.json`) with the already-present `serde_json` dep. Claude Code's `settings.json` may carry `//` line comments (JSONC); either strip line comments before parsing or treat a parse failure as safe-degrade (no candidate, no error) — both are acceptable, pick the simpler. Value must be absolute or `~/`-prefixed; expand a leading `~/` via `HOME`. Emit one extra candidate (`basis=claude_auto_memory_dir`, `native_project=None`, `coverage=Full`) when it resolves to an existing UTF-8 dir; dedup against `projects/*` candidates by canonicalized path. `autoMemoryDirectory` relocates where Claude *writes* auto-memory, so it is a peer discovery root, not a project-scoped setting. `ponytail:` project-scope `.claude/settings.json` is intentionally not read — Tessera has no project context at discovery time.
- **`coverage_level=Full`** is honest capability disclosure of Claude Code's official surface (fully enumerable Markdown), declared ahead of 2.2's enumeration — the same pattern Codex used in Story 1.2. Applies to both the `projects/*` and `autoMemoryDirectory` candidates.
- **Multi-candidate-per-provider is new.** Codex emits 0..1 candidate; Claude emits 0..N (one per project, plus optionally `autoMemoryDirectory`). `discover_sources`' union handles this and the UI already renders a list. Each root gets its own fingerprint/`source_id`.
- **Dispatch widening is unavoidable.** With two providers the `Option<CodexAdapter>` return shape can no longer stay concrete; `Option<Box<dyn ProviderAdapter>>` is the chosen mechanism (matches the architecture's "adapter registry" language).
- **Rescan message consistency.** `ProviderNotScannable` is an *expected* 2.1 outcome (Claude parsing is 2.2), so it must not look like an `internal` failure on any surface. Give it a dedicated `error_code` and provider-aware reason text, and branch on it in the rescan worker (the sync `/api/scan` envelope already does).
- **`enumerate_*` hard-fail.** Returning `Err` (not empty `Ok`) for `claude_code` ensures that if the scan guard is ever bypassed, a misrouted scan fails loudly instead of committing an empty generation as a false success.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- expected: all existing tests pass plus new `claude_code_discover` and extended confirm/rescan-message tests.
- `cargo test --manifest-path server/Cargo.toml claude_code` -- expected: Claude discover (incl. `autoMemoryDirectory`) + capability-honesty + enumerate-hard-fail tests green.
- `cargo test --manifest-path server/Cargo.toml provider_not_scannable` -- expected: the dedicated error code + provider-aware message asserted on every surface.
- `cargo test --manifest-path server/Cargo.toml confirm_failed_for_unknown_provider` -- expected: passes; `claude_code` is in the allow-set.
- `npm run build` -- expected: TS compiles with the extended `DiscoveryBasis` union; no type errors.
- `npx playwright test tests/ui/accessibility.spec.ts` -- expected: accessibility contract green and the provider-agnostic copy asserted.
- `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings` -- expected: clean.

**Manual checks:**
- With a throwaway `autoMemoryDirectory` in `~/.claude/settings.json` pointing at a temp memory dir, confirm Tessera discovers it as a `claude_auto_memory_dir` candidate alongside the project candidates; confirm/reject it; verify the dir's files are byte/mtime-identical afterward. Remove the throwaway setting after.

## Auto Run Result

Status: done
Follow-up review recommended: true (pass-2 patches: medium 3, low 6 → score 15 ≥ 5).

**Summary:** Claude Code is now a second Agent Memory provider (discovery + confirmation only) alongside Codex. Discovery covers both inputs named by the story AC — the official `$CLAUDE_CONFIG_DIR/projects/<project>/memory/` layout (one candidate per project) AND a user-configured `autoMemoryDirectory` read from the user-scope `<config_dir>/settings.json` (absolute or `~/`-prefixed, `~/` expanded via HOME, deduped by canonicalized path, safe-degrade on missing/unparseable/invalid). Provider dispatch is widened from mono-Codex to a `Box<dyn ProviderAdapter>` registry; `claude_code` routes through the reused canonicalize→fingerprint→upsert pipeline; a `ProviderNotScannable` guard keeps the Codex parser off Claude files until 2.2 and surfaces a provider-aware message on every surface; `enumerate_*` hard-fails for Claude. TS `DiscoveryBasis` + UI copy are generalized.

**Files changed:** `server/src/adapters/claude_code.rs`, `server/src/adapters/codex.rs`, `server/src/application/source.rs`, `server/src/application/scan.rs`, `server/src/domain/ports/provider_adapter.rs`, `server/src/domain/scan.rs`, `server/src/http/envelope.rs`, `server/src/http/mod.rs`, `server/tests/claude_code_discover.rs` (new), `server/tests/source_registry.rs`, `server/tests/http_api.rs`, `src/api/discover.ts`, `src/features/sources/Sources.tsx`, `tests/ui/accessibility.spec.ts`, `playwright.config.ts`.

**Review findings (pass 2):** patches applied 9 (medium 3, low 6); deferred 3 (recorded in `deferred-work.md`); rejected 4 (noise/covered). Pass 1's intent gap (`autoMemoryDirectory`) is resolved; pass 1's patches were folded in and verified to hold.

**Verification performed:** `cargo test --manifest-path server/Cargo.toml` → 226 passed, 0 failed, 1 ignored (pre-existing); `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings` → clean; `npm run build` → TS+vite clean; `npx playwright test tests/ui/accessibility.spec.ts` → 3 passed (incl. the new TS-basis render test). Matrix Test Audit: all I/O-matrix rows (projects + `autoMemoryDirectory` + confirm/reject/disable + rescan-message-consistency) covered by passing tests.

**Residual risks:** Claude parsing/indexing is Story 2.2 — confirmed Claude sources show coverage=Full, record count 0, health=unknown until then (rescan fails safely with a provider-aware message). `autoMemoryDirectory` is read at user scope only (project-scope settings are not consulted — Tessera has no project context at discovery). Three low-severity items deferred (see Review Triage Log pass 2).

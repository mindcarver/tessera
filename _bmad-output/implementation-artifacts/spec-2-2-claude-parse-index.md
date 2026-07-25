---
title: 'Story 2.2: Claude Code memory parsing, boundary restriction & read-only indexing'
type: 'feature'
created: '2026-07-25'
status: 'done'
baseline_revision: '1cc34e3'
review_loop_iteration: 0
followup_review_recommended: true
context:
  - '{project-root}/_bmad-output/implementation-artifacts/epic-2-context.md'
  - '{project-root}/_bmad-output/implementation-artifacts/spec-2-1-claude-discover.md'
warnings: ['oversized']
---

<intent-contract>

## Intent

**Problem:** Story 2.1 brought Claude Code on board as a discoverable, confirmable provider, but deliberately blocked scanning: a `ProviderNotScannable` guard refuses to scan any `claude_code` source, and `ClaudeCodeAdapter::enumerate_*` hard-fails. So confirmed Claude sources can never be indexed or searched.

**Approach:** Implement Claude Code's parsing + read-only indexing by (a) giving `ClaudeCodeAdapter` a real `enumerate_*` that parses the flat `memory/` dir (`MEMORY.md` + topic `*.md`) into canonical records, reusing the generic Markdown parser Codex already uses; (b) generalizing the scan pipeline's two hard-coded Codex entry points to dispatch adapter + parser by `source.provider`; and (c) removing the 2.1 `ProviderNotScannable` guard (and its three HTTP surfaces) now that Claude is scannable. Claude records flow through Epic 1's atomic generational pipeline unchanged.

## Boundaries & Constraints

**Always:**
- Index every direct-child `*.md` of the confirmed Claude `memory/` dir (`MEMORY.md` + topic Markdown). **No recursion, no subdirectory walking** (verified real layout: flat, `.md`-only, no subdirs).
- Canonical records carry `provider = "claude_code"`, `parser_version = "claude-markdown/v1"`, identity from the shared Markdown parser's heading/section `native_unit_id` (file-level unit when a file has no headings), and `native_locator` = `file_uri(absolute_path)` + line-range fragment — the same locator scheme Codex uses.
- Reuse Epic 1's atomic generational pipeline verbatim (staging generation → fencing-token CAS commit → `dirty_after_validation` never activates; boot reconcile). Claude records coexist with Codex records; `record_id` is already provider-parameterized.
- Scan dispatch is by `source.provider`: the production entry points (`scan_source`, `scan_reserved_source`) select the adapter (and its parser + version) instead of hard-coding `&CodexAdapter`. Codex behavior and wire contract are unchanged.
- Zero source mutation (NFR-1); memory file **bodies** are read only inside the parser (NFR-5). `MEMORY.md` is tagged as the index memory type and topic `*.md` as a distinct topic type (honest display for 2.3/2.4 filtering).

**Never:**
- Recurse into subdirectories of `memory/`; index non-`*.md` files; index `CLAUDE.md` / `AGENTS.md` / `.claude/rules` / session / transcript content, or anything in a manually-added subdirectory. These surface as `unsupported_artifact` diagnostics, never as records (A-18).
- Mutate any Claude source file (zero-write).
- Write a second Markdown parser for Claude. `canonicalize_markdown` (in `codex.rs` today) is a generic heading/section parser with no Codex semantics — reuse it (extract to a shared module if clean), only the `parser_version` tag differs.
- Re-block Claude behind `ProviderNotScannable`. That guard was a 2.1 placeholder; 2.2 removes it (variant + `error_code` + the three HTTP surfaces + the `safe_error_reason`/`health_for_scan_error` arms) and replaces the 2.1 pinned tests with real Claude scan tests.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Typical project memory | `memory/` has `MEMORY.md` + N topic `*.md` | One canonical record set per file (`provider=claude_code`, `parser_version=claude-markdown/v1`, heading/section ids, file locator); generation activates | No error |
| Index-only | `memory/` has only `MEMORY.md` | `MEMORY.md` parsed into canonical units; activates | No error |
| Empty memory dir | `memory/` exists, no files | Zero records; scan completes and activates an empty generation | No error |
| Reject instruction files | `memory/` also contains `CLAUDE.md`, `AGENTS.md` | Those two are rejected by name (`unsupported_artifact` diagnostic); the `*.md` memory files still index | No error |
| Reject non-markdown / subdirs | `memory/` contains a `.json`/`.txt` and a subdir | Non-`*.md` and subdir → `unsupported_artifact` diagnostics; never indexed, never recursed | No error |
| Symlink escape | a `*.md` child symlinks outside the canonical root | That child is skipped (realpath containment check, mirroring Codex) | No error |
| `autoMemoryDirectory` source | confirmed source rooted at a user `autoMemoryDirectory` dir | Same parsing/activation as a project `memory/` dir | No error |
| Cross-provider coexistence | one Codex + one Claude source confirmed | Both scan independently; records carry their own `provider`/`parser_version`; query sees both | No error |
| Rescan idempotent | unchanged Claude source rescanned | Same `record_id`s, same content; generation swaps atomically | No error |
| Enumeration failure | `memory/` unreadable (permissions) | `EnumerateError`; run fails with enumeration code; previous active generation unchanged | Structured error |

</intent-contract>

## Code Map

- `server/src/adapters/claude_code.rs` — replace the `Err` stubs in `enumerate_file_units`/`enumerate_artifacts` with real flat-`*.md` enumeration (allowlist + rejecter diagnostics + symlink-escape check); reuse the shared Markdown parser; declare `claude-markdown/v1`.
- `server/src/adapters/codex.rs` (+ a shared `server/src/adapters/markdown.rs` if extracted) — `canonicalize_markdown` + `file_uri`/`percent_encode_fragment`/`safe_relative_path` are generic; extract or re-export so both adapters share them without duplication.
- `server/src/domain/ports/provider_adapter.rs` — add a `parser_version()` member to `ProviderAdapter` (or expose a per-adapter const the orchestrator reads); add `ProviderMemoryType::TopicMemory` for Claude topic files (`Memory` = `MEMORY.md` index).
- `server/src/application/scan.rs` — remove the `CODEX_PROVIDER_ID`/`ProviderNotScannable` guards (`:147-149`, `:235-238`); make `scan_source`/`scan_reserved_source` dispatch the adapter by `source.provider` (e.g. an `adapter_for_scan(provider) -> Option<Box<dyn ProviderAdapter>>` paralleling `application::source::adapter_for`); read `parser_version` from the adapter instead of the hard-coded `CODEX_MARKDOWN_PARSER_VERSION` (`:347`).
- `server/src/domain/scan.rs`, `server/src/http/envelope.rs`, `server/src/http/mod.rs` — remove `ScanError::ProviderNotScannable`, its `error_code` arm, `PROVIDER_NOT_SCANNABLE_MSG`, `scan_failed_provider_not_scannable`, the rescan-SSE branch, the `map_scan_error` arm, and the `safe_error_reason`/`health_for_scan_error` arms.
- `server/tests/scan_pipeline.rs` (or a new `server/tests/claude_code_scan.rs`) — Claude's five-class contract tests via the existing scripted-adapter seam.
- `server/tests/fixtures/providers/claude_code/` — Claude fixture(s): a `memory/` dir with `MEMORY.md` + topic `*.md` + rejecter `CLAUDE.md`/non-`.md`; plus a parser-boundary fixture mirroring `codex/canonical-boundaries.md`.
- `server/tests/performance_baseline.rs` — re-run the Codex perf gate post-refactor to prove no regression (the gate is Codex-fixture-pinned; a Claude perf baseline is optional/YAGNI).
- `server/tests/source_registry.rs`, `server/tests/http_api.rs`, `server/tests/claude_code_discover.rs` — replace the 2.1 `ProviderNotScannable`-pinned tests with real Claude scan/rescan tests.

## Tasks & Acceptance

**Execution:**
- `server/src/domain/ports/provider_adapter.rs` -- add `parser_version()` to the `ProviderAdapter` trait and `ProviderMemoryType::TopicMemory`; default/implement `parser_version()` on both adapters (`codex-markdown/v1`, `claude-markdown/v1`) -- single source of truth for the persisted parser version, replacing the hard-coded constant.
- `server/src/adapters/claude_code.rs` -- implement `enumerate_file_units`/`enumerate_artifacts`: direct-child `*.md` of `memory/` only (no recursion); `MEMORY.md`→`Memory`, other `*.md`→`TopicMemory`; reject `CLAUDE.md`/`AGENTS.md` by name and non-`*.md`/subdirs as `unsupported_artifact` diagnostics; apply the same realpath containment (symlink-escape) check Codex uses -- the Supported Artifact Matrix (A-18) boundary.
- `server/src/adapters/codex.rs` (+ `markdown.rs` if extracted) -- lift `canonicalize_markdown` and the path helpers to a shared location and have both adapters call them -- one Markdown parser, two version tags; no behavior change to Codex parsing.
- `server/src/application/scan.rs` -- delete the `ProviderNotScannable` guards and the `CODEX_PROVIDER_ID` alias; dispatch the adapter by `source.provider` in `scan_source`/`scan_reserved_source` (rejecting genuinely-unknown providers with a clear error, not the removed variant); read `parser_version` from the adapter at the record-build site -- generalize the pipeline without changing the atomic generation/CAS semantics.
- `server/src/domain/scan.rs` + `server/src/http/{envelope,mod}.rs` -- remove `ScanError::ProviderNotScannable`, its `error_code`, `PROVIDER_NOT_SCANNABLE_MSG`, `scan_failed_provider_not_scannable`, the rescan-SSE branch, and the `map_scan_error`/`safe_error_reason`/`health_for_scan_error` arms -- the 2.1 placeholder surface is gone now that Claude scans.
- `server/tests/fixtures/providers/claude_code/` -- add a `memory/` fixture (`MEMORY.md` + ≥2 topic `*.md` + a rejecter `CLAUDE.md` + a non-`.md`) and a parser-boundary fixture -- anchors for the contract tests.
- `server/tests/scan_pipeline.rs` (or new `claude_code_scan.rs`) -- add Claude's five classes: fixture-contract (heading/section + rejecter boundaries), zero-source-mutation, parser-version (`claude-markdown/v1`), reconcile-recovery (stale-run + drift), capability-honesty (empty dir, symlink dedup/honest count, enumeration failure) -- mirror the Codex classes via the scripted-adapter seam.
- `server/tests/{source_registry,http_api,claude_code_discover}.rs` -- replace the 2.1 `ProviderNotScannable`-pinned tests (`source_registry.rs:695-801`, `http_api.rs:717`, `claude_code_discover.rs:580`) with tests asserting Claude sources actually scan/activate and are searchable.
- `server/tests/performance_baseline.rs` -- re-run the Codex perf gate after the dispatch refactor and record that it still passes (regression report for the same Phase 0 fixture; no new Claude baseline required unless the gate demands one).

**Acceptance Criteria:**
- Given a confirmed Claude source whose `memory/` has `MEMORY.md` + topic `*.md`, when it is scanned, then canonical records are produced with `provider="claude_code"`, `parser_version="claude-markdown/v1"`, heading/section `native_unit_id`s, and file locators; the generation activates and the records are returnable by the query service.
- Given a `memory/` containing `CLAUDE.md`/`AGENTS.md`, a non-`*.md` file, or a subdirectory, when canonicalization runs, then those are rejected (`unsupported_artifact` diagnostics) and never indexed, while the legitimate `*.md` memory files still index (A-18).
- Given a confirmed Claude source, when a scan runs (and a rescan runs), then no Claude source file's content/size/mtime changes (NFR-1 zero-write).
- Given a Claude source, when its enumeration fails (e.g. unreadable dir), then the run fails with an enumeration error code and the previous active generation is preserved.
- Given the five adapter-contract classes, when the Claude suite runs, then fixture-contract, zero-source-mutation, parser-version, reconcile-recovery, and capability-honesty all pass for `claude_code` (A-13).
- Given a confirmed source rooted at a user `autoMemoryDirectory`, when it is scanned, then it parses/activates identically to a project `memory/` dir (autoMemoryDirectory resolves correctly end-to-end).
- Given a Codex source, when scanned after the dispatch refactor, then its records, `parser_version`, and behavior are unchanged; and the Phase 0 perf gate still passes (no regression).
- Given the codebase, then no `ProviderNotScannable` / `provider_not_scannable` / `PROVIDER_NOT_SCANNABLE_MSG` references remain (the 2.1 placeholder is fully removed).

## Spec Change Log

## Review Triage Log

### 2026-07-25 — Review pass
- intent_gap: 0
- bad_spec: 0
- patch: 8: (high 0, medium 4, low 4)
- defer: 1: (low 1)
- reject: 0
- addressed_findings:
  - `[medium]` `[patch]` Claude enumerate rejecter + `MEMORY.md` role tag now case-insensitive (`eq_ignore_ascii_case`) — on case-insensitive volumes a `claude.md`/`memory.md` no longer leaks instruction text into the index or gets mis-tagged as `TopicMemory`.
  - `[medium]` `[patch]` One provider→adapter registry (`adapter_for_scan` delegates to `application::source::adapter_for`); test asserts both agree for codex/claude_code/unknown — confirm and scan dispatch can no longer drift.
  - `[medium]` `[patch]` `scan_reserved_source` now `fail_run`s the reserved run row on unknown-provider dispatch (was leaving it for boot recovery, breaking the fail-on-error contract).
  - `[medium]` `[patch]` Cross-provider search test added — scans Codex + Claude and asserts `application::search` returns both providers' records (exercises the FTS5 read path the AC names, not just raw SQL).
  - `[low]` `[patch]` `debug_assert_eq!(adapter.provider_id(), source.provider)` guards the pub scan seam against adapter/source mismatch (defense lost when the 2.1 guard was removed).
  - `[low]` `[patch]` `scan_source` no longer double-reads the source row (passes the loaded `Source` through to `scan_source_with`).
  - `[low]` `[patch]` `EmptyScanWithActiveGeneration` rescan-to-empty path now tested for Claude.
  - `[low]` `[patch]` `parser_version()` carries the persisted tag (single source of truth); verified perf-gate logic was NOT touched.
- deferred (see `_bmad-output/implementation-artifacts/deferred-work.md`): the Phase 0 perf gate's tight, machine-calibrated threshold can false-fail under parallel `cargo test` load or on slower machines — pre-existing test infra, not introduced by 2.2; clock-independent "no Codex behavioral regression" is already proven by `codex_canonicalization` (parser-output pin) + the cross-coexistence dispatch test.

## Design Notes

- **One Markdown parser, two tags.** `canonicalize_markdown` (`codex.rs:453`) handles fences, ATX/setext headings, preamble, and nested-section ids with no Codex semantics. Claude's `MEMORY.md`/topic files are the same CommonMark-ish shape, so reuse it — extract to `adapters::markdown` (or re-export) and tag Claude records `claude-markdown/v1`. A separate tag lets a future Claude-specific grammar bump trigger a reparse without touching Codex identity.
- **Dispatch, not a second pipeline.** `scan_source_with<A>` is already generic; only `scan_source`/`scan_reserved_source` and the record-build site (`parser_version`, the imported parser) hard-code Codex. Add a provider→adapter lookup (mirroring `application::source::adapter_for`) and move `parser_version` onto the adapter. Everything else — staging, fencing-token CAS, reconcile — is shared and must not be duplicated.
- **Boundary mirrors Codex's discipline.** Claude's `memory/` is flat `*.md` (verified on 18 real dirs). Enumerate direct children only: `MEMORY.md`+topic `*.md` index; `CLAUDE.md`/`AGENTS.md`/non-`*.md`/subdirs become `unsupported_artifact` diagnostics; realpath containment skips escaping symlinks. This is the same shape as Codex's root-file + one-level-dir rule.
- **`ProviderNotScannable` is deleted, not repurposed.** It existed only because 2.1 deferred Claude parsing. With Claude scannable it has no caller; removing the variant + its 3 HTTP surfaces + the reason/health arms is cleaner than leaving dead defense. Unknown-future providers that genuinely cannot scan should fail via `EnumerateError`/enumeration-failed, not a special pre-enumeration variant.
- **Perf gate is regression-only.** The gate pins a Codex anonymized fixture with 2× thresholds; 2.2 re-runs it to prove generic dispatch didn't regress Codex. A Claude perf baseline is YAGNI unless a later story needs one.
- **Continuity from 2.1 (KEEP).** `ClaudeCodeAdapter` discovery (`projects/*/memory/` + user-scope `autoMemoryDirectory`), `CLAUDE_CONFIG_DIR` priority, multi-candidate-per-project, the `Box<dyn ProviderAdapter>` confirm dispatch, and the `ClaudeAutoMemoryDir` basis all survive — 2.2 only adds enumeration/parsing and removes the scan block.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- expected: all existing tests pass plus new Claude scan/contract tests; the removed `ProviderNotScannable` tests are gone.
- `cargo test --manifest-path server/Cargo.toml claude` -- expected: Claude enumerate + scan + contract tests green.
- `cargo test --manifest-path server/Cargo.toml performance_baseline` -- expected: the Codex Phase 0 perf gate still passes (no regression from generic dispatch).
- `npm run build` -- expected: TS compiles (no UI change required, but confirm no breakage).
- `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings` -- expected: clean.

**Manual checks:**
- Confirm a real Claude source (e.g. an existing `~/.claude/projects/<P>/memory/` with `MEMORY.md` + topics), scan it via the app, and verify records appear in search with `provider=claude_code` and full provenance — and that the dir's files are byte/mtime-identical afterward.

## Auto Run Result

Status: done
Follow-up review recommended: true (pass patches: medium 4, low 4 → score 16 ≥ 5).

**Summary:** Claude Code is now a fully scannable provider. `ClaudeCodeAdapter::enumerate_*` parses the flat `memory/` dir (`MEMORY.md` → `Memory`, topic `*.md` → `TopicMemory`; rejects `CLAUDE.md`/`AGENTS.md`/non-`*.md`/subdirs as `unsupported_artifact`; symlink-escape contained, case-insensitive matching). The shared Markdown parser was extracted to `adapters/markdown.rs` (one parser, two version tags — `codex-markdown/v1`, `claude-markdown/v1` via a new `parser_version()` trait method). The scan pipeline dispatches adapter + parser by `source.provider` through one unified registry. The 2.1 `ProviderNotScannable` guard and its three HTTP surfaces are fully removed. Claude records flow through Epic 1's atomic generational pipeline unchanged and are returnable by the query service (FTS5).

**Files changed:** `server/src/adapters/{claude_code,codex,markdown(new),mod}.rs`, `server/src/application/{scan,source}.rs`, `server/src/domain/{ports/provider_adapter,scan}.rs`, `server/src/http/{envelope,mod}.rs`, `server/tests/{claude_code_scan(new),claude_code_discover,http_api,scan_pipeline,source_registry}.rs`, `server/tests/fixtures/providers/claude_code/{canonical-boundaries.md,memory/{MEMORY.md,python-patterns.md,rust-notes.md,CLAUDE.md,session-notes.json}}`.

**Review findings:** patches applied 8 (medium 4, low 4); deferred 1 (perf-gate hardening — pre-existing infra); rejected 0.

**Verification:** `cargo test` (skip flaky perf gate) → 248 passed, 0 failed; isolated perf gate → 8 passed (cold_scan 6ms, no Codex regression); `codex_canonicalization` 11/11 pins parser output post-extraction; cross-coexistence test routes Codex via production dispatch with `codex-markdown/v1`; `cargo clippy --all-targets -D warnings` → clean; `npm run build` → clean.

**Residual risks:** (1) The perf gate's tight machine-calibrated threshold can false-fail under parallel load / slower machines — pre-existing test infra, deferred for hardening; clock-independent regression proof is in place. (2) `ProviderMemoryType` has no TS mirror yet — lands in 2.3/2.4 with cross-provider search/filter UI. (3) `scan_diagnostics` (rejected-artifact records) is write-only — no HTTP/inventory/query read path yet; surfacing boundary rejections to the UI is a 2.3/2.4 concern. (4) `parser_version()` is a required trait method (no default) — future providers must implement it (compile-time enforced).

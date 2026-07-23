# Edge Case Hunter — Tessera Story 1.4

Work in `/Users/carver/workspace/mindcarver/tessera`. Do not modify files.

Invoke the `bmad-review` skill with only the `edge-case-hunter` lens on the supplied review material.

Review material is a full-file audit rather than a Git diff: the story records `baseline_revision: NO_VCS`. Treat the following current files as the code under review and read each in full:

- `server/src/index/migrations.rs`
- `server/src/domain/scan.rs`
- `server/src/domain/mod.rs`
- `server/src/domain/ports/provider_adapter.rs`
- `server/src/adapters/codex.rs`
- `server/src/index/scan_store.rs`
- `server/src/index/mod.rs`
- `server/src/application/scan.rs`
- `server/src/application/mod.rs`
- `server/src/http/envelope.rs`
- `server/src/http/mod.rs`
- `server/src/http/server.rs`
- `server/src/lib.rs`
- `server/tests/scan_pipeline.rs`
- `src/api/scan.ts`
- `src/api/errors.ts`
- `src/features/sources/Sources.tsx`

Spec: `_bmad-output/implementation-artifacts/spec-1-4-scan-pipeline.md`.

Focus on failure paths, TOCTOU, stale ownership, source-boundary changes, empty-versus-unreadable roots, malformed persisted state, API input edge cases, and UI state races. Return only substantiated findings as a Markdown list. For every finding include: severity, exact `file:line`, violated requirement or invariant, a concise failure scenario, and concrete evidence.

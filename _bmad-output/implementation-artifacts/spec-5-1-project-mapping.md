---
title: 'Story 5.1: Tessera Project Creation and Native Project Explicit Mapping'
type: 'feature'
created: '2026-07-26'
status: 'done'
baseline_revision: '39cd2de0f1773182e007d7d649e82a83492dd516'
final_revision: '6f5c26caf50eea4791a27e78e9194286137eeeb3'
review_loop_iteration: 0
followup_review_recommended: true
context:
  - '{project-root}/CLAUDE.md'
  - '{project-root}/_bmad-output/implementation-artifacts/epic-5-context.md'
warnings:
  - oversized
---

<intent-contract>

## Intent

**Problem:** Tessera can already preserve each Agent Memory's provider-native `native_project` (FR-4) and expose a reserved-but-empty `tessera_project` filter slot (Story 2.4), but there is no way for Carver to create a Tessera Project and explicitly associate multiple Native Projects (across Codex and Claude Code) into one local cross-Agent view. Without this mapping layer, the Epic 5 federation goal — one view per real-world project — has no foundation.

**Approach:** Add a local-only Tessera Project mapping layer in the Rust core: a new `domain::project` module, a `project_store` persistence layer, an application service that enforces explicit-only mapping with AD-27 cardinality, and six loopback HTTP endpoints under the existing versioned (`api_version: "1"`) contract. Surface it in the React UI as a keyboard-reachable Projects region (create / rename / delete / add-mapping / remove-mapping) mirroring the existing inline-confirm pattern. Mapping targets are `(provider, native_project)` pairs — the same native identity already carried on Sources and canonical records — so projection (Story 5.2) can filter records later without copying them.

## Boundaries & Constraints

**Always:**
- Mappings live only in Tessera's own SQLite (new `tessera_projects` + `project_mappings` tables). Provider directories/files are never read-for-write or written.
- Zero-source-mutation gate: source file set/content/size/mtime unchanged before/after every project or mapping operation.
- AD-27 cardinality: within one mapping scope `(provider, native_project)`, a Native Project belongs to at most one active Tessera Project. Re-adding the exact same `(project, provider, native_project)` is idempotent; claiming a scope already owned by another project is rejected, never silently moved.
- AD-24 no-auto-merge: creating a project creates zero mappings; unmapped Native Projects are never auto-projected; only explicit `mappings/add` forms an association.
- NFR-3 redaction: error messages and logs contain no memory body, query text, or credentials (project name + provider + native_project are already user-visible metadata, consistent with the Source Inventory).
- api_version stays `"1"` (additive only — no existing endpoint contract changes). New tables are `STRICT`, snake_case columns, enums as lowercase TEXT, timestamps as `INTEGER` Unix seconds. Reuse the existing `unix_seconds_now_i64()` helper and `unchecked_transaction()` + `with_transaction` seam. Loopback-only (`127.0.0.1`); `PRAGMA foreign_keys = ON`.

**Block If:** none. All decisions below are resolvable unattended.

**Never:**
- Do NOT fill or enable the reserved `tessera_project` search filter (`SearchFilters.tessera_project`, `Search.tsx` reserved control) — that is Story 5.2.
- Do NOT add `project_mapping_revision` or bind project state into the search/browse cursor (AD-26/AD-31) — Story 5.2.
- Do NOT implement projection (browse/search by Tessera Project) — Story 5.2.
- Do NOT auto-merge scopes, auto-project unmapped entries, or silently override an existing mapping.
- Do NOT delete or modify canonical `memory_records` or `source_registry` rows on any project/mapping operation.
- Do NOT introduce new crate/npm dependencies (locked stack: `rusqlite`, `tiny_http`, `serde`; React only). No router, no CSS framework.

## I/O & Edge-Case Matrix

All POST bodies are JSON; responses are `Envelope<T>` with `api_version: "1"`. `NativeProjectRef = { provider: string, native_project: string|null }`. `TesseraProjectView = { project_id: string, name: string, created_at: i64, updated_at: i64, mappings: NativeProjectRef[] }`.

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Create project | `POST /api/projects/create` `{name:"A"}` | `200` `Envelope<TesseraProjectView>` `{project_id:"proj_<n>", name:"A", mappings:[]}` | `400 bad_request` phase `project` if name empty/whitespace or >128 chars |
| List projects | `GET /api/projects` (none exist) | `200` `Envelope<[]>` (ordered by `id` ascending) | none |
| Rename project | `POST /api/projects/rename` `{project_id:"proj_1", name:"B"}` | `200` updated view; `updated_at` advanced | `404 project_not_found`; `400 bad_request` invalid name |
| Add mapping (happy) | `POST /api/projects/mappings/add` `{project_id, provider:"claude_code", native_project:"<key>"}` | `200` view with the mapping present | `404 project_not_found`; `400 bad_request` unknown provider, or `native_project` empty/whitespace or >1024 chars |
| Cardinality conflict (AD-27) | add `(claude_code,"<key>")` to project B while already mapped to A | `409 mapping_conflict`; message names owning project A; **no row created** | `mapping_conflict` |
| Idempotent re-add | re-add exact `(project_id, provider, native_project)` already in same project | `200` unchanged view; no duplicate row | none |
| Codex global scope | add `(provider:"codex", native_project:null)` | stored; at most one `(codex,null)` across all projects | `409 mapping_conflict` if a second project claims it |
| Remove mapping | `POST /api/projects/mappings/remove` `{project_id, provider, native_project}` | `200` view without that mapping | `404 project_not_found`; `404 mapping_not_found` |
| Delete project | `POST /api/projects/delete` `{project_id}` | `200` `{project_id, removed_mappings:<count>}`; mappings cascade | `404 project_not_found` |

Provider must be one of `codex`, `claude_code` (lowercase). `native_project` is `null` for Codex's global store or a **non-empty, non-whitespace** string ≤1024 chars for Claude Code project keys — empty/whitespace strings are rejected as `bad_request` so they cannot collide with the Codex `null` scope under the `COALESCE` uniqueness index. Within a `TesseraProjectView`, `mappings` are ordered by `id` ascending. The backend does NOT require a matching confirmed Source — mappings are user-authored state stable across rebind/rebuild (AD-29/AD-33).

</intent-contract>

## Code Map

- `server/src/index/migrations.rs` -- add migration id `7` `v6_tessera_projects` (DDL below); the runner uses `MIGRATIONS` (highest current id `6`).
- `server/src/index/mod.rs` -- `CURRENT_SCHEMA_VERSION` const is stale (`5` while max migration id is `6`); update to `7` to match the new max id and re-export `ProjectStore`.
- `server/src/domain/project.rs` (NEW) -- `ProjectId(pub String)` with `from_rowid`/`to_rowid` (`proj_<n>`); `TesseraProject`, `TesseraProjectView`, `NativeProjectRef`, `ProjectMapping`; request DTOs (`CreateProjectRequest`, `RenameProjectRequest`, `DeleteProjectRequest`, `MappingRequest`); validation constants (`MAX_PROJECT_NAME_LEN=128`, `MAX_NATIVE_PROJECT_LEN=1024`, `KNOWN_PROVIDERS`).
- `server/src/domain/mod.rs` -- re-export `project`.
- `server/src/index/project_store.rs` (NEW) -- `ProjectStore<'a> { conn }` mirroring `SourceRegistry<'a>`: `create / list / get / rename / delete / add_mapping / remove_mapping` + `with_transaction`. Owns all SQL for the two new tables.
- `server/src/application/project.rs` (NEW) -- orchestration mirroring `application/source.rs`: input validation, cardinality pre-check → `mapping_conflict` (resolves owning project name), idempotent re-add detection, transactional writes, DTO assembly (project + its mappings → `TesseraProjectView`).
- `server/src/application/mod.rs` -- re-export the project service functions.
- `server/src/http/envelope.rs` -- add `ErrorEnvelope` constructors `project_not_found`, `mapping_conflict(owning_name)`, `mapping_not_found`; add phase constant `"project"`. (HTTP status mapping: `project_not_found`/`mapping_not_found` → 404, `mapping_conflict` → 409, `bad_request` → 400, else 500 — extend the existing status mapper.)
- `server/src/http/mod.rs` -- handlers `create_project / list_projects / rename_project / delete_project / add_mapping / remove_mapping` (signature shape: request DTO + `&IndexState` → `Result<Envelope<T>, ErrorEnvelope>`); re-export them.
- `server/src/http/server.rs` -- six new arms in the `match (method, path)` route table; parse JSON POST bodies the same way existing `/api/sources/confirm`/`rebind` do.
- `src/api/projects.ts` (NEW) -- typed client `createProject / listProjects / renameProject / deleteProject / addMapping / removeMapping` + DTOs, using `apiGet`/`apiPost` from `src/api/client.ts` and the same runtime shape-guarding as other modules.
- `src/api/errors.ts` -- add `mapping_conflict`, `project_not_found`, `mapping_not_found` to `TESSERA_STABLE_ERROR_CODES` so their safe messages render.
- `src/features/projects/Projects.tsx` (NEW) -- Projects region: list (name + mapped Native Projects), create form, rename, delete (inline-confirm region mirroring Story 4.4 rebuild: `aria-expanded` trigger, focus moved in, Esc cancels), add-mapping (provider + native_project picker fed by `getSourceInventory()`), remove-mapping. `aria-live` status, `data-testid` hooks.
- `src/App.tsx` -- render a new `<Projects>` `<section aria-label>` alongside the existing regions; leave the hand-rolled `View` union intact.
- `tests/ui/accessibility.spec.ts` -- add a Playwright case asserting the Projects flow is keyboard-reachable; leave the existing case that pins the reserved Tessera-project filter slot disabled **unchanged and still passing**.

## Tasks & Acceptance

**Execution:**
- `server/src/index/migrations.rs` -- add migration `id:7, name:"v6_tessera_projects"` creating `tessera_projects`, `project_mappings`, and the scope uniqueness index (DDL in Design Notes) -- foundational schema for the mapping layer.
- `server/src/index/mod.rs` -- set `CURRENT_SCHEMA_VERSION = 7` (fixing the stale `5`) and re-export `ProjectStore` -- keeps the version const honest and wired.
- `server/src/domain/project.rs` (NEW) + `server/src/domain/mod.rs` -- define `ProjectId` (`proj_<n>`), DTOs, request structs, and validation constants; re-export -- the canonical types every other layer consumes.
- `server/src/index/project_store.rs` (NEW) + `server/src/index/mod.rs` -- implement `ProjectStore<'a>` CRUD + `with_transaction` over the two tables -- the persistence boundary (only place that touches project SQL).
- `server/src/application/project.rs` (NEW) + `server/src/application/mod.rs` -- validation, cardinality/idempotency logic, transactional orchestration, DTO assembly -- the only orchestrator for project ops (AD-1).
- `server/src/http/envelope.rs` -- add the three error constructors + `"project"` phase and extend the status mapper -- structured, redacted errors for the new surface.
- `server/src/http/mod.rs` + `server/src/http/server.rs` -- six handlers + route arms + JSON body parsing -- exposes the versioned, loopback-only contract.
- Rust tests (co-located per existing convention) -- migration applies + `schema_version == "7"`; create/list/get/rename/delete; add/remove; cardinality conflict returns `mapping_conflict` and creates no row; idempotent re-add; Codex-null scope uniqueness; unknown-provider/invalid-name `bad_request`; non-destruction (`source_registry` + active `memory_records` counts unchanged across a flurry of project ops); zero-source-mutation (source sizes/mtimes unchanged).
- `src/api/projects.ts` (NEW) + `src/api/errors.ts` -- typed client + allowlist the new error codes -- UI can call the contract and surface safe messages.
- `src/features/projects/Projects.tsx` (NEW) + `src/App.tsx` -- keyboard-reachable Projects region wired into the shell -- the user-facing create/rename/delete + add/remove-mappings interaction (UX-DR3 dev-stage details).
- `tests/ui/accessibility.spec.ts` -- a11y case for the Projects flow; keep reserved-slot case green -- AD-21/NFR-13 keyboard contract holds and 5.2's slot stays reserved.

**Acceptance Criteria:**
- Given no Tessera Projects exist, when `POST /api/projects/create {name:"A"}` is called, then the response is `200` with `project_id` matching `^proj_\d+$`, `name:"A"`, equal `created_at`/`updated_at`, and an empty `mappings` array.
- Given project "A" exists, when `GET /api/projects` is called, then the response is `200` with an array containing A's view (and no mappings appear that were never explicitly added — AD-24 no-auto-merge).
- Given project "A", when `POST /api/projects/rename {project_id, name:"B"}` is called, then the response is `200` with `name:"B"` and `updated_at` strictly greater than `created_at`; renaming an unknown id returns `404 project_not_found`.
- Given projects "A" and "B" and a confirmed Claude Code source with `native_project:"<key>"`, when `<key>` is added to A then to B, then the A add returns `200` and the B add returns `409 mapping_conflict` whose message names A; a subsequent `GET /api/projects` shows `<key>` mapped only to A (no row was created for B).
- Given `<key>` is already mapped to A, when the identical `(A, claude_code, <key>)` add is repeated, then the response is `200` and A's `mappings` still contains exactly one entry for `<key>` (idempotent, no duplicate).
- Given a Codex global source (`native_project:null`), when `(codex, null)` is added to A then to B, then the B add returns `409 mapping_conflict` (at most one active project per `(codex, null)` scope — AD-27 with NULL collapsed).
- Given `<key>` mapped to A, when `POST /api/projects/mappings/remove {A, claude_code, <key>}` is called, then the response is `200` with `<key>` absent from A; removing a non-existent mapping returns `404 mapping_not_found`.
- Given project A with N mappings, when `POST /api/projects/delete {A}` is called, then the response is `200` with `removed_mappings == N`, and a subsequent `GET /api/projects` no longer lists A (mappings cascade; project gone).
- Given any sequence of create/rename/delete/add/remove operations, when `GET /api/sources/inventory` and a search are re-run, then record counts, health, and results are identical to before (mapping ops never delete or modify canonical records or sources — non-destruction).
- Given the operation sequence above, when source file sizes and mtimes are compared before and after, then they are unchanged (zero-source-mutation; mappings never touch Provider files).
- Given a keyboard-only user, when they tab through the Projects region, then they can create, rename, delete (via the inline-confirm), add a mapping, and remove a mapping entirely via keyboard, status changes are announced via `aria-live`, and the reserved "Tessera project" filter control in Search remains `disabled` (Story 5.2 slot untouched).
- Given the implementation, when `cargo test --manifest-path server/Cargo.toml` and `npm run build` are run, then both succeed (new Rust tests green; new TypeScript compiles; pre-existing Windows-only Unix test failures ignored per project memory).

## Spec Change Log

<!-- Empty until the first bad_spec loopback from step-04. -->

## Review Triage Log

### 2026-07-26 — Review pass
- intent_gap: 0
- bad_spec: 0
- patch: 10: (high 0, medium 0, low 10)
- defer: 2: (high 0, medium 0, low 2)
- reject: 9: (high 0, medium 0, low 9)
- addressed_findings:
  - `[low]` `[patch]` A2/A3: `ProjectStore::delete` relied on the FK cascade only with an unverified pre-count — now deletes mappings explicitly (cascade-independent) and reports the actual removed count (`server/src/index/project_store.rs`).
  - `[low]` `[patch]` A6: `unix_seconds_now_i64` was duplicated — `scan_store`'s is now `pub(crate)` and reused in `project_store` (spec said "reuse").
  - `[low]` `[patch]` A7: removed unrelated `rescan_events` drive-by re-export from `server/src/lib.rs`.
  - `[low]` `[patch]` V5: wire cardinality test now proves "no row created for B" via delete-B `removed_mappings==0` (was a substring-only check).
  - `[low]` `[patch]` V6: empty-name 400 wire test now asserts stable `code` + `phase` (was status-only).
  - `[low]` `[patch]` V1: added add-mapping `404 project_not_found` wire test (I/O matrix row 4 error path was untested).
  - `[low]` `[patch]` V2: added list-projects id-ascending order assertion (I/O matrix row 2 promise).
  - `[low]` `[patch]` IA-a: added test that one project holds both `(codex,null)` and `(claude_code,"<key>")` mappings.
  - `[low]` `[patch]` V4/IA-g: non-destruction test now also pins `scan_runs` + `scan_diagnostics` counts and byte-identical `GET /api/sources/inventory` before/after the flurry.
  - `[low]` `[patch]` V7: added wire cases for the remaining shape-validation branches (claude empty/whitespace `native_project`, codex non-null, over-length name).
  - Deferred (not addressed here, logged to deferred-work): A8 (disabled + `aria-expanded` on the Delete trigger — pre-existing 4.4 house pattern); E1/A5 (UI `maxLength` UTF-16 vs backend UTF-8 bytes for multi-byte names).
  - Notable rejects: A1 (delete test asserts `count(project_mappings)==0` and passes — reviewer's "cannot pass" empirically false); A4 (UNIQUE race-backstop unreachable under the single-writer `IndexState` mutex, spec-tolerated); V3 (filesystem zero-source-mutation is vacuous — no project path performs file I/O); IA-d/e/f (HTTP/UI associable-set divergence is by design; "active" lifecycle and projection are deferred to 5.2 / unsupported by intent).

## Design Notes

**Why `(provider, native_project)` is the mapping target, not `source_id`.** FR-5 associates *Native Projects*, and canonical records already carry `(provider, native_project)`; keying the mapping on that pair keeps it stable across a Source rebind (AD-33: rebind re-derives `native_project` from the new root, so the mapping key survives) and lets Story 5.2 project records with a direct `(provider, native_project) IN (...)` predicate — no copy of canonical rows, no native-identity change (AD-2). The "mapping scope" in AD-27 is therefore `(provider, native_project)`.

**Why "adjust mapping target" is remove+add, not a move endpoint.** AD-27 forbids silently overriding an existing mapping. Remove-then-add is fully explicit and avoids a half-built move primitive; cardinality conflict cannot occur because the scope is freed before re-adding. `updated_at` reflects project-metadata changes (rename) only; mapping rows carry their own `created_at`.

**Golden schema (migration id 7, `v6_tessera_projects`):**
```sql
CREATE TABLE tessera_projects (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE project_mappings (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    tessera_project_id INTEGER NOT NULL REFERENCES tessera_projects(id) ON DELETE CASCADE,
    provider           TEXT    NOT NULL,
    native_project     TEXT,
    created_at         INTEGER NOT NULL
) STRICT;

-- AD-27: at most one active project per (provider, native_project).
-- COALESCE collapses NULL (Codex global) to '' so NULL scopes are unique too.
CREATE UNIQUE INDEX project_mappings_scope_unique
    ON project_mappings (provider, COALESCE(native_project, ''));
```
The scope index alone enforces both cross-project cardinality and same-project idempotency; the application pre-checks `SELECT tessera_project_id … WHERE provider=? AND COALESCE(native_project,'')=COALESCE(?,?)` inside the transaction to return `mapping_conflict` naming the owner (rather than surfacing a raw constraint violation), with the index as the concurrency backstop.

**Deferred to Story 5.2 (record in deferred-work):** fill the reserved `tessera_project` filter with a real SQL predicate; introduce `project_mapping_revision` and bind it into the `Cursor`/`BrowseCursor` snapshot (AD-26/AD-31) so a mapping change invalidates in-flight pagination via `stale_snapshot`; projection browse/search by Tessera Project.

## Verification

**Commands:**
- `cargo test --manifest-path server/Cargo.toml` -- expected: new project/store/application/migration tests pass; the ~20 pre-existing Unix-only failures are ignored on Windows (project memory). Assert `schema_version == "7"` post-migration.
- `npm run build` -- expected: TypeScript compiles cleanly with the new `src/api/projects.ts` and `src/features/projects/Projects.tsx`.
- `npm run test:e2e` (Playwright) -- expected: the new Projects a11y case passes and the existing reserved-Tessera-project-filter case still passes (slot remains disabled). Run focused if the full suite is too heavy.

**Manual checks (if Playwright cannot run headless here):**
- Boot the core, create a project + two mappings via `curl` against `127.0.0.1:1420`, attempt a conflicting mapping and confirm `409 mapping_conflict`, then delete the project and confirm `GET /api/sources/inventory` is byte-identical to the pre-operation snapshot.

## Auto Run Result

Status: done

**Summary:** Implemented Story 5.1 — Tessera Project creation and Native Project explicit mapping. A new `domain::project` + `index::project_store` + `application::project` layer adds six loopback HTTP endpoints (`/api/projects/{create,list,rename,delete}` + `/api/projects/mappings/{add,remove}`) under the existing `api_version: "1"` contract, persisted in two new SQLite tables (`tessera_projects`, `project_mappings`) via migration id 7. AD-27 cardinality (one active project per `(provider, native_project)` scope, NULL collapsed via `COALESCE`) is enforced by a unique index plus an application pre-check returning `mapping_conflict`; AD-24 no-auto-merge holds (create → zero mappings; explicit-only). A keyboard-reachable React Projects region mirrors the existing inline-confirm pattern. Story 5.2 work (fill the reserved `tessera_project` filter, `project_mapping_revision` cursor binding, projection) is intentionally deferred.

**Files changed:**
- New: `server/src/domain/project.rs`, `server/src/index/project_store.rs`, `server/src/application/project.rs`, `server/tests/projects_api.rs`, `src/api/projects.ts`, `src/features/projects/Projects.tsx`, `_bmad-output/implementation-artifacts/epic-5-context.md`.
- Modified: `server/src/index/migrations.rs` (+id 7), `server/src/index/mod.rs` (`CURRENT_SCHEMA_VERSION` 7), `server/src/domain/mod.rs`, `server/src/application/mod.rs`, `server/src/http/envelope.rs`, `server/src/http/mod.rs`, `server/src/http/server.rs`, `server/src/lib.rs`, `server/tests/rebuild.rs`, `server/tests/scan_pipeline.rs` (schema_version 6→7), `src/App.tsx`, `src/api/errors.ts`, `tests/ui/accessibility.spec.ts`.

**Review findings (this pass):** patches applied 10 (all low); deferred 2; rejected 9. Follow-up review recommended: true (10 low patches → score 10 ≥ 5). See `## Review Triage Log` for the per-finding breakdown.

**Verification performed:**
- `cargo test --manifest-path server/Cargo.toml --test projects_api` → 14 passed, 0 failed.
- `cargo test --manifest-path server/Cargo.toml --lib application::project` → 10 passed; `http::envelope` 9; `index::migrations` 4; `--test rebuild` 13; `--test scan_pipeline migrations_apply` 1.
- `npm run build` → TypeScript + Vite clean.
- Playwright (focused): Projects a11y + 409 safe-alert + reserved-filter-still-disabled → all green.
- Pre-existing ~20 Windows-only Unix test failures unchanged (project memory); no new failures.

**Residual risks:**
- Story 5.2 must fill the reserved `tessera_project` filter, add `project_mapping_revision` to the cursor snapshot (AD-26/AD-31), and implement projection (recorded in Design Notes + `deferred-work.md`).
- The add-mapping picker offers only native projects present in the current Source Inventory (refresh on inventory change); the HTTP API remains permissive by design.
- Uncommitted `_bmad/` BMAD-installer artifacts (config + `.bak`) are unrelated to this story and excluded from the commit.

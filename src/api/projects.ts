/**
 * Tessera — typed TS clients for Tessera Project create / list / rename /
 * delete + add-mapping / remove-mapping (Story 5.1).
 *
 * Mirrors the Rust types in `server/src/domain/project.rs`
 * (ProjectId / TesseraProjectView / NativeProjectRef / request DTOs). Must
 * stay in lock-step with the Rust side: any field rename / removal / type
 * change here is a contract break that requires an `api_version` bump
 * (AD-17/A-6).
 *
 * Architecture invariants honored by this module:
 * - **AD-1 application boundary:** the UI never touches SQLite or the
 *   filesystem; every call goes through the loopback HTTP API.
 * - **AD-24 explicit-only mapping:** `createProject` carries no mappings;
 *   only `addMapping` forms an association.
 * - **AD-27 cardinality:** the backend returns 409 `mapping_conflict` when a
 *     scope is already owned by another project; the safe message (named in
 *     `TESSERA_STABLE_ERROR_CODES`) renders verbatim.
 * - **Versioned envelopes:** every response is validated against
 *   `API_VERSION`; any drift throws `TesseraApiError` with code
 *   `api_contract` (Phase 0 review finding: never fabricate a fake success).
 * - **Honest narrow types:** enums are literal unions of the stable wire
 *   strings, matching Rust's `#[serde(rename_all = "snake_case")]` (no enum
 *   to widen yet — `provider` is a `string` because the backend vocabulary
 *   `codex` | `claude_code` is also surfaced by the Source Inventory).
 */

import { apiGet, apiPost, API_VERSION, type Envelope, type TesseraApiError } from "./client";

// Re-export the shared envelope / error shapes so feature code can import
// everything from one place.
export type { Envelope, TesseraApiError };

// ---------------------------------------------------------------------------
// Project domain mirror (Rust: domain::project)
// ---------------------------------------------------------------------------

/**
 * Opaque stable Tessera Project handle (`proj_<n>`). The numeric portion is
 * the `tessera_projects` table's AUTOINCREMENT rowid and is never reused
 * (Story 5.1 ships `deleteProject`, so a deleted project's handle cannot be
 * reattached to a different row).
 */
export type ProjectId = string;

/**
 * A `(provider, native_project)` reference — the mapping target. This is the
 * same native identity already carried on Sources and canonical records, so
 * Story 5.2 projection can filter records with a direct predicate (AD-2: no
 * copy of canonical rows, no native-identity change).
 *
 * `native_project` is `null` for Codex's global store; for Claude Code it is
 * the provider-native project key (a non-empty, non-whitespace string).
 */
export interface NativeProjectRef {
  provider: string;
  native_project: string | null;
}

/**
 * The DTO returned to the UI for a Tessera Project. Carries the project row
 * plus its ordered mappings (ordered by `id` ascending on the backend for
 * stable UI rendering).
 */
export interface TesseraProjectView {
  project_id: ProjectId;
  name: string;
  /** Unix-epoch seconds at project creation. */
  created_at: number;
  /** Unix-epoch seconds at the most recent rename. Equal to created_at until
   * the first rename advances it. */
  updated_at: number;
  mappings: NativeProjectRef[];
}

/**
 * `POST /api/projects/delete` response: the deleted project's id + the count
 * of mappings that cascaded with it (per the spec I/O matrix).
 */
export interface DeleteProjectResponse {
  project_id: ProjectId;
  removed_mappings: number;
}

// ---------------------------------------------------------------------------
// Request DTOs (mirror the Rust endpoints)
// ---------------------------------------------------------------------------

export interface CreateProjectRequest {
  name: string;
}

export interface RenameProjectRequest {
  project_id: ProjectId;
  name: string;
}

export interface DeleteProjectRequest {
  project_id: ProjectId;
}

export interface MappingRequest {
  project_id: ProjectId;
  provider: string;
  native_project: string | null;
}

// ---------------------------------------------------------------------------
// Runtime shape guards
// ---------------------------------------------------------------------------

function isString(v: unknown): v is string {
  return typeof v === "string";
}

function isNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

function isOptionalStringOrNull(v: unknown): boolean {
  return v === null || typeof v === "string";
}

/**
 * Runtime shape guard for a single `NativeProjectRef`. Returns the narrowed
 * value or `null` so callers throw a structured contract error rather than
 * passing bad data into React state. Pinning every field — not just
 * `typeof === "object"` — means a Rust-side rename (e.g. `provider` →
 * `provider_id`) surfaces as a contract error instead of an undefined-render
 * bug.
 */
function asNativeProjectRef(value: unknown): NativeProjectRef | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (!isString(v.provider) || !isOptionalStringOrNull(v.native_project)) {
    return null;
  }
  return {
    provider: v.provider,
    native_project: v.native_project as string | null,
  };
}

/**
 * Runtime shape guard for a single `TesseraProjectView`. Returns the narrowed
 * value or `null`.
 */
function asTesseraProjectView(value: unknown): TesseraProjectView | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (
    !isString(v.project_id) ||
    !isString(v.name) ||
    !isNumber(v.created_at) ||
    !isNumber(v.updated_at) ||
    !Array.isArray(v.mappings)
  ) {
    return null;
  }
  const mappings: NativeProjectRef[] = [];
  for (const m of v.mappings) {
    const ref = asNativeProjectRef(m);
    if (ref === null) return null;
    mappings.push(ref);
  }
  return {
    project_id: v.project_id,
    name: v.name,
    created_at: v.created_at,
    updated_at: v.updated_at,
    mappings,
  };
}

/** Throw a structured API contract error (Phase 0 review finding). */
function throwContractError(message: string): never {
  throw {
    code: "api_contract",
    message,
    source_id: null,
    phase: "transport",
  } satisfies TesseraApiError;
}

// ---------------------------------------------------------------------------
// Clients (mirror the six Rust endpoints)
// ---------------------------------------------------------------------------

/**
 * `POST /api/projects/create`. AD-24: the response carries an empty
 * `mappings` array — creating a project creates zero mappings.
 */
export async function createProject(
  name: string,
): Promise<Envelope<TesseraProjectView>> {
  const envelope = (await apiPost("/api/projects/create", {
    name,
  } as CreateProjectRequest)) as Envelope<TesseraProjectView> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    asTesseraProjectView(envelope.payload) !== null
  ) {
    return {
      api_version: envelope.api_version,
      payload: asTesseraProjectView(envelope.payload) as TesseraProjectView,
    };
  }
  throwContractError(
    "Tessera core create_project response did not match the versioned envelope contract.",
  );
}

/**
 * `GET /api/projects`. Returns the views ordered by `id` ascending (the
 * backend's stable ordering). Empty array when no projects exist.
 */
export async function listProjects(): Promise<Envelope<TesseraProjectView[]>> {
  const envelope = (await apiGet("/api/projects")) as Envelope<TesseraProjectView[]> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    Array.isArray(envelope.payload) &&
    envelope.payload.every((p) => asTesseraProjectView(p) !== null)
  ) {
    return {
      api_version: envelope.api_version,
      payload: envelope.payload.map((p) => asTesseraProjectView(p) as TesseraProjectView),
    };
  }
  throwContractError(
    "Tessera core list_projects response did not match the versioned envelope contract.",
  );
}

/**
 * `POST /api/projects/rename`. Advances `updated_at`; unknown id → 404
 * `project_not_found` (a stable error code the UI renders verbatim).
 */
export async function renameProject(
  projectId: ProjectId,
  name: string,
): Promise<Envelope<TesseraProjectView>> {
  const envelope = (await apiPost("/api/projects/rename", {
    project_id: projectId,
    name,
  } as RenameProjectRequest)) as Envelope<TesseraProjectView> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    asTesseraProjectView(envelope.payload) !== null
  ) {
    return {
      api_version: envelope.api_version,
      payload: asTesseraProjectView(envelope.payload) as TesseraProjectView,
    };
  }
  throwContractError(
    "Tessera core rename_project response did not match the versioned envelope contract.",
  );
}

/**
 * `POST /api/projects/delete`. Cascades the project's mappings; the response
 * carries the cascade count via `removed_mappings`.
 */
export async function deleteProject(
  projectId: ProjectId,
): Promise<Envelope<DeleteProjectResponse>> {
  const envelope = (await apiPost("/api/projects/delete", {
    project_id: projectId,
  } as DeleteProjectRequest)) as Envelope<DeleteProjectResponse> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    envelope.payload &&
    typeof envelope.payload === "object" &&
    isString((envelope.payload as unknown as Record<string, unknown>).project_id) &&
    isNumber((envelope.payload as unknown as Record<string, unknown>).removed_mappings)
  ) {
    return {
      api_version: envelope.api_version,
      payload: envelope.payload as DeleteProjectResponse,
    };
  }
  throwContractError(
    "Tessera core delete_project response did not match the versioned envelope contract.",
  );
}

/**
 * `POST /api/projects/mappings/add`. AD-27 cardinality: a scope already owned
 * by another project returns 409 `mapping_conflict` naming the owner; re-
 * adding the same scope to the same project is idempotent.
 *
 * `nativeProject` is `null` for Codex's global store and a non-empty string
 * for Claude Code (validation lives on the backend; the TS client passes the
 * value through verbatim).
 */
export async function addMapping(
  projectId: ProjectId,
  provider: string,
  nativeProject: string | null,
): Promise<Envelope<TesseraProjectView>> {
  const envelope = (await apiPost("/api/projects/mappings/add", {
    project_id: projectId,
    provider,
    native_project: nativeProject,
  } as MappingRequest)) as Envelope<TesseraProjectView> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    asTesseraProjectView(envelope.payload) !== null
  ) {
    return {
      api_version: envelope.api_version,
      payload: asTesseraProjectView(envelope.payload) as TesseraProjectView,
    };
  }
  throwContractError(
    "Tessera core add_mapping response did not match the versioned envelope contract.",
  );
}

/**
 * `POST /api/projects/mappings/remove`. Distinguishes "no such project"
 * (404 `project_not_found`) from "project exists, no such mapping" (404
 * `mapping_not_found`); the UI renders each safe message verbatim.
 */
export async function removeMapping(
  projectId: ProjectId,
  provider: string,
  nativeProject: string | null,
): Promise<Envelope<TesseraProjectView>> {
  const envelope = (await apiPost("/api/projects/mappings/remove", {
    project_id: projectId,
    provider,
    native_project: nativeProject,
  } as MappingRequest)) as Envelope<TesseraProjectView> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    asTesseraProjectView(envelope.payload) !== null
  ) {
    return {
      api_version: envelope.api_version,
      payload: asTesseraProjectView(envelope.payload) as TesseraProjectView,
    };
  }
  throwContractError(
    "Tessera core remove_mapping response did not match the versioned envelope contract.",
  );
}

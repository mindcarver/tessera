/**
 * Tessera — Obsidian Knowledge (Phase C.0) API client (Story 6.2/6.3/6.6).
 *
 * Mirrors the Rust Knowledge pipeline (`adapters::obsidian`, the
 * `/api/knowledge/*` endpoints). Knowledge is a separate domain from Agent
 * Memory (AD-19/AD-38): it routes through independent endpoints and an
 * independent canonical table, never through `ProviderAdapter`. This module
 * keeps the same versioned-envelope contract as the other clients.
 *
 * Re-exports shared transport types so features import from one place.
 */

export type { Envelope, TesseraApiError } from "./client";
export type { CandidateSource } from "./discover";

import { apiGet, apiPost, API_VERSION } from "./client";
import type { Envelope, TesseraApiError } from "./client";

// --- shared validation helpers (local copies; sources.ts keeps its own) -----

function isString(v: unknown): v is string {
  return typeof v === "string";
}

function isNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

function isOptionalStringOrNull(v: unknown): boolean {
  return v === null || typeof v === "string";
}

const VALID_HEALTH_STATES: ReadonlySet<string> = new Set([
  "unknown",
  "healthy",
  "degraded",
  "error",
]);
const VALID_LIFECYCLES: ReadonlySet<string> = new Set([
  "confirmed",
  "disabled",
  "rejected",
]);
const VALID_HEALTH_CAUSES: ReadonlySet<string> = new Set([
  "none",
  "path_missing",
  "permission_denied",
  "format_unsupported",
  "scan_failed",
]);
const VALID_COVERAGE_LEVELS: ReadonlySet<string> = new Set([
  "full",
  "search_only",
  "existence_only",
  "unsupported",
]);
const VALID_DISCOVERY_BASES: ReadonlySet<string> = new Set([
  "default_home",
  "codex_home_env",
  "claude_default_home",
  "claude_config_dir_env",
  "claude_auto_memory_dir",
  "obsidian_vault_registry",
]);

function throwContractError(message: string): never {
  throw {
    code: "api_contract",
    message,
    source_id: null,
    phase: "transport",
  } satisfies TesseraApiError;
}

// --- Story 6.2 — Knowledge discovery ----------------------------------------

/** A discovered Obsidian Vault candidate (mirrors Rust `CandidateSource`). */
export interface KnowledgeCandidate {
  provider: string;
  root_path: string;
  basis: string;
  coverage_level: string;
  native_project: string | null;
}

/** The Knowledge discovery payload: candidates plus a registry diagnostic. */
export interface KnowledgeDiscoveryPayload {
  candidates: KnowledgeCandidate[];
  /** Stable diagnostic code, or null when the registry parsed cleanly. */
  diagnostic: string | null;
}

function asKnowledgeCandidate(value: unknown): KnowledgeCandidate | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (
    !isString(v.provider) ||
    !isString(v.root_path) ||
    !(typeof v.basis === "string" && VALID_DISCOVERY_BASES.has(v.basis)) ||
    !(typeof v.coverage_level === "string" && VALID_COVERAGE_LEVELS.has(v.coverage_level)) ||
    !isOptionalStringOrNull(v.native_project)
  ) {
    return null;
  }
  return v as unknown as KnowledgeCandidate;
}

export async function discoverKnowledgeSources(): Promise<
  Envelope<KnowledgeDiscoveryPayload>
> {
  const raw = (await apiGet("/api/knowledge/discover")) as unknown;
  const envelope = raw as Envelope<unknown> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    envelope.payload &&
    typeof envelope.payload === "object"
  ) {
    const payload = envelope.payload as Record<string, unknown>;
    if (
      Array.isArray(payload.candidates) &&
      payload.candidates.every((c) => asKnowledgeCandidate(c) !== null) &&
      isOptionalStringOrNull(payload.diagnostic)
    ) {
      return {
        api_version: envelope.api_version,
        payload: {
          candidates: payload.candidates.map(
            (c) => asKnowledgeCandidate(c) as KnowledgeCandidate,
          ),
          diagnostic: payload.diagnostic as string | null,
        },
      };
    }
  }
  throwContractError(
    "Tessera core Knowledge discovery response did not match the versioned envelope contract.",
  );
}

// --- Story 6.3 — confirm / reject / Rust-owned vault picker -----------------

/** The outcome of the Rust-owned native folder picker (mirrors Rust enum). */
export type VaultPickerOutcome =
  | { status: "selected"; candidate: KnowledgeCandidate }
  | { status: "cancelled" }
  | { status: "invalid" };

function asVaultPickerOutcome(value: unknown): VaultPickerOutcome | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (v.status === "cancelled" || v.status === "invalid") {
    return { status: v.status };
  }
  if (v.status === "selected") {
    const candidate = asKnowledgeCandidate(v.candidate);
    if (candidate) return { status: "selected", candidate };
  }
  return null;
}

export async function requestVaultPicker(): Promise<Envelope<VaultPickerOutcome>> {
  const envelope = (await apiPost("/api/knowledge/picker", {})) as Envelope<VaultPickerOutcome> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    asVaultPickerOutcome(envelope.payload) !== null
  ) {
    return {
      api_version: envelope.api_version,
      payload: asVaultPickerOutcome(envelope.payload) as VaultPickerOutcome,
    };
  }
  throwContractError(
    "Tessera core vault-picker response did not match the versioned envelope contract.",
  );
}

export async function confirmKnowledgeSource(
  candidate: KnowledgeCandidate,
): Promise<Envelope<unknown>> {
  // The Rust handler wraps a Source DTO; the UI does not need its full shape
  // for the Inventory view, so we validate only the envelope + api_version.
  const envelope = (await apiPost("/api/knowledge/confirm", { candidate })) as Envelope<unknown> | null;
  if (envelope && envelope.api_version === API_VERSION) {
    return envelope;
  }
  throwContractError(
    "Tessera core Knowledge confirm response did not match the versioned envelope contract.",
  );
}

export async function rejectKnowledgeSource(
  candidate: KnowledgeCandidate,
): Promise<Envelope<unknown>> {
  const envelope = (await apiPost("/api/knowledge/reject", { candidate })) as Envelope<unknown> | null;
  if (envelope && envelope.api_version === API_VERSION) {
    return envelope;
  }
  throwContractError(
    "Tessera core Knowledge reject response did not match the versioned envelope contract.",
  );
}

// --- Story 6.6 — Knowledge Inventory ----------------------------------------

/**
 * A Knowledge (Obsidian Vault) Inventory row. Mirrors Rust
 * `domain::scan::KnowledgeInventory`. Parallel to `SourceInventory` but for
 * `local_knowledge` Sources; carries the supported-note count from the
 * independent `knowledge_records` table (AD-38).
 */
export interface KnowledgeInventory {
  source_id: string;
  vault_name: string;
  provider: string;
  root: string;
  coverage_level: string;
  health_state: string;
  last_successful_scan: number | null;
  /** Supported Markdown-note count when full coverage + active generation. */
  complete_note_count: number | null;
  latest_error: string | null;
  cause: string | null;
  stale: boolean;
  lifecycle_state: string;
}

function asKnowledgeInventory(value: unknown): KnowledgeInventory | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (
    !isString(v.source_id) ||
    !isString(v.vault_name) ||
    !isString(v.provider) ||
    !isString(v.root) ||
    !(typeof v.coverage_level === "string" && VALID_COVERAGE_LEVELS.has(v.coverage_level)) ||
    !(typeof v.health_state === "string" && VALID_HEALTH_STATES.has(v.health_state)) ||
    !(v.last_successful_scan === null || isNumber(v.last_successful_scan)) ||
    !(v.complete_note_count === null || isNumber(v.complete_note_count)) ||
    !isOptionalStringOrNull(v.latest_error) ||
    !(v.cause === null || (typeof v.cause === "string" && VALID_HEALTH_CAUSES.has(v.cause))) ||
    typeof v.stale !== "boolean" ||
    !(typeof v.lifecycle_state === "string" && VALID_LIFECYCLES.has(v.lifecycle_state))
  ) {
    return null;
  }
  return v as unknown as KnowledgeInventory;
}

export async function getKnowledgeInventory(): Promise<Envelope<KnowledgeInventory[]>> {
  const envelope = (await apiGet("/api/knowledge/inventory")) as Envelope<KnowledgeInventory[]> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    Array.isArray(envelope.payload) &&
    envelope.payload.every((item) => asKnowledgeInventory(item) !== null)
  ) {
    return {
      api_version: envelope.api_version,
      payload: envelope.payload.map((item) => asKnowledgeInventory(item) as KnowledgeInventory),
    };
  }
  throwContractError(
    "Tessera core Knowledge Inventory response did not match the versioned envelope contract.",
  );
}

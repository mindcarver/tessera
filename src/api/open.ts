import { API_VERSION, apiPost, type Envelope } from "./client";

export interface OpenResult {
  record_id: string;
  source_id: string;
}

export async function openOriginalLocation(recordId: string): Promise<Envelope<OpenResult>> {
  const value = await apiPost("/api/open", { record_id: recordId });
  if (!isOpenEnvelope(value)) throw apiContractError();
  return value;
}

function isOpenEnvelope(value: unknown): value is Envelope<OpenResult> {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  if (v.api_version !== API_VERSION || !v.payload || typeof v.payload !== "object") return false;
  const payload = v.payload as Record<string, unknown>;
  return typeof payload.record_id === "string" && typeof payload.source_id === "string";
}

function apiContractError(): never {
  throw { code: "api_contract", message: "Tessera core returned an unsupported open response.", source_id: null, phase: "open" };
}

/**
 * Tessera — Obsidian Knowledge Inventory view (Story 6.6 / Phase C.0).
 *
 * The Knowledge-domain counterpart to `features/sources/Sources.tsx`. Renders:
 *  - discovered Obsidian Vault candidates (from the host registry), each with
 *    independent confirm/reject;
 *  - a Rust-owned native folder-picker fallback ("选择已有 Vault");
 *  - one Inventory card per confirmed Vault showing the supported-note count,
 *    coverage, health, last-success scan, stale state, and safe latest error.
 *
 * Domain separation (AD-19): this view never mixes Agent Memory. It calls only
 * the `/api/knowledge/*` endpoints and renders Knowledge candidates/inventory
 * exclusively. All copy is inline zh-CN (mirrors Sources.tsx localization
 * posture).
 */
import { useCallback, useEffect, useState, type ReactElement } from "react";
import {
  browseKnowledge,
  confirmKnowledgeSource,
  discoverKnowledgeSources,
  getKnowledgeInventory,
  rejectKnowledgeSource,
  requestVaultPicker,
  searchKnowledge,
  type KnowledgeCandidate,
  type KnowledgeInventory,
  type KnowledgeNoteResult,
  type KnowledgeSearchResult,
} from "../../api/obsidian";
import { readTesseraErrorMessage } from "../../api/errors";
import type { HealthState } from "../../api/sources";
import { HealthPill } from "../../components/ui/HealthPill";

type LoadState<T> =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ok"; value: T };

/** Registry diagnostic code → human zh-CN explanation. */
function describeDiagnostic(diagnostic: string | null): string | null {
  switch (diagnostic) {
    case "registry_missing":
      return "未找到 Obsidian 注册表（可能尚未安装或从未打开 Obsidian）。可手动选择已有 Vault。";
    case "registry_unreadable":
      return "Obsidian 注册表无法读取。可手动选择已有 Vault。";
    case "registry_corrupt":
      return "Obsidian 注册表格式异常，部分 Vault 可能未显示。可手动选择已有 Vault。";
    default:
      return null;
  }
}

function describeCoverage(level: string): string {
  switch (level) {
    case "full":
      return "完整覆盖";
    case "search_only":
      return "仅可搜索";
    case "existence_only":
      return "仅确认存在";
    case "unsupported":
      return "不支持";
    default:
      return level;
  }
}

function describeHealth(state: string): string {
  switch (state) {
    case "healthy":
      return "正常";
    case "degraded":
      return "降级";
    case "error":
      return "错误";
    case "unknown":
    default:
      return "未知";
  }
}

function scanText(epochSeconds: number | null): string {
  if (epochSeconds === null) return "尚未扫描";
  const date = new Date(epochSeconds * 1000);
  return `上次扫描 ${date.toLocaleString("zh-CN")}`;
}

function noteCountText(count: number | null): string {
  if (count === null) return "—";
  return `${count} 篇笔记`;
}

function asHealthState(s: string): HealthState {
  return s as HealthState;
}

/** A single confirmed-Vault Inventory card. */
function VaultCard({
  item,
  onRefresh,
  onBrowse,
}: {
  item: KnowledgeInventory;
  onRefresh: () => void;
  onBrowse: () => void;
}): ReactElement {
  const attention = item.health_state === "degraded" || item.health_state === "error";
  return (
    <li
      data-provider={item.provider}
      data-health={item.health_state}
      className={`tsr-card${attention ? " tsr-card--deg" : ""}`}
    >
      <article className="tsr-card__body">
        <div className="tsr-card__row">
          <div className="tsr-card__main">
            <h5 className="tsr-card__prov">Obsidian 知识库</h5>
            <div className="tsr-card__name">{item.vault_name}</div>
            <code className="tsr-card__path">{item.root}</code>
            <div className="tsr-card__meta">
              {describeCoverage(item.coverage_level)} · {scanText(item.last_successful_scan)}
            </div>
          </div>
          <div className="tsr-card__count" aria-label="笔记数量">
            {noteCountText(item.complete_note_count)}
          </div>
          <div className="tsr-card__aside">
            <HealthPill state={asHealthState(item.health_state)} compact label={describeHealth(item.health_state)} />
            <div className="tsr-card__actions">
              {item.complete_note_count !== null && item.complete_note_count > 0 ? (
                <button type="button" className="tsr-btn tsr-btn--primary" onClick={onBrowse}>
                  浏览笔记
                </button>
              ) : null}
              <button type="button" className="tsr-btn" onClick={onRefresh}>
                刷新
              </button>
            </div>
          </div>
        </div>
        {item.stale ? (
          <p className="tsr-card__stale" role="status">
            当前显示的是上次成功扫描的结果（已标记为过期）。
          </p>
        ) : null}
        {item.latest_error ? (
          <p className="tsr-card__error" role="alert">
            {item.latest_error}
          </p>
        ) : null}
      </article>
    </li>
  );
}

export function Obsidian(): ReactElement {
  const [discovery, setDiscovery] = useState<
    LoadState<{ candidates: KnowledgeCandidate[]; diagnostic: string | null }>
  >({ kind: "loading" });
  const [inventory, setInventory] = useState<LoadState<KnowledgeInventory[]>>({ kind: "loading" });
  const [message, setMessage] = useState("");
  const [pickerBusy, setPickerBusy] = useState(false);
  // Story 6.9 — Browse view state: which vault is being browsed + its notes.
  const [browseVault, setBrowseVault] = useState<{ sourceId: string; vaultName: string } | null>(
    null,
  );
  const [browseNotes, setBrowseNotes] = useState<LoadState<KnowledgeNoteResult[]>>({
    kind: "loading",
  });
  const [browseCursor, setBrowseCursor] = useState<string | null>(null);
  const browsePageSize = 20;
  // Story 6.9 — Search state.
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<LoadState<KnowledgeSearchResult[]>>({
    kind: "loading",
  });
  const [searchCursor, setSearchCursor] = useState<string | null>(null);
  const [searchActive, setSearchActive] = useState(false);

  const refreshInventory = useCallback(() => {
    getKnowledgeInventory()
      .then((result) => setInventory({ kind: "ok", value: result.payload }))
      .catch((error: unknown) =>
        setInventory({ kind: "error", message: readTesseraErrorMessage(error) }),
      );
  }, []);

  const refreshDiscovery = useCallback(() => {
    discoverKnowledgeSources()
      .then((result) =>
        setDiscovery({
          kind: "ok",
          value: { candidates: result.payload.candidates, diagnostic: result.payload.diagnostic },
        }),
      )
      .catch((error: unknown) =>
        setDiscovery({ kind: "error", message: readTesseraErrorMessage(error) }),
      );
  }, []);

  useEffect(() => {
    refreshDiscovery();
    refreshInventory();
  }, [refreshDiscovery, refreshInventory]);

  // Story 6.9 — load the first page of notes for a vault.
  const loadBrowse = useCallback(
    (sourceId: string, vaultName: string) => {
      setBrowseVault({ sourceId, vaultName });
      setBrowseCursor(null);
      setBrowseNotes({ kind: "loading" });
      browseKnowledge(sourceId, browsePageSize)
        .then((page) => {
          setBrowseNotes({ kind: "ok", value: page.payload.results });
          setBrowseCursor(page.payload.next_cursor);
        })
        .catch((error: unknown) =>
          setBrowseNotes({ kind: "error", message: readTesseraErrorMessage(error) }),
        );
    },
    [browsePageSize],
  );

  // Load more notes (next page), appending to the existing list.
  const loadMoreNotes = useCallback(() => {
    if (!browseVault || !browseCursor) return;
    const cursor = browseCursor;
    browseKnowledge(browseVault.sourceId, browsePageSize, cursor)
      .then((page) => {
        setBrowseNotes((prev) =>
          prev.kind === "ok"
            ? { kind: "ok", value: [...prev.value, ...page.payload.results] }
            : prev,
        );
        setBrowseCursor(page.payload.next_cursor);
      })
      .catch((error: unknown) => setMessage(readTesseraErrorMessage(error)));
  }, [browseVault, browseCursor]);

  const exitBrowse = useCallback(() => {
    setBrowseVault(null);
    setBrowseNotes({ kind: "loading" });
    setBrowseCursor(null);
    refreshInventory();
  }, [refreshInventory]);

  // Story 6.9 — keyword search across all confirmed vaults.
  const runSearch = useCallback((q: string) => {
    if (!q.trim()) return;
    setSearchActive(true);
    setSearchQuery(q);
    setSearchCursor(null);
    setSearchResults({ kind: "loading" });
    searchKnowledge(q, browsePageSize)
      .then((page) => {
        setSearchResults({ kind: "ok", value: page.payload.results });
        setSearchCursor(page.payload.next_cursor);
      })
      .catch((error: unknown) =>
        setSearchResults({ kind: "error", message: readTesseraErrorMessage(error) }),
      );
  }, [browsePageSize]);

  const loadMoreSearch = useCallback(() => {
    if (!searchCursor || !searchQuery) return;
    const cursor = searchCursor;
    searchKnowledge(searchQuery, browsePageSize, cursor)
      .then((page) => {
        setSearchResults((prev) =>
          prev.kind === "ok" ? { kind: "ok", value: [...prev.value, ...page.payload.results] } : prev,
        );
        setSearchCursor(page.payload.next_cursor);
      })
      .catch((error: unknown) => setMessage(readTesseraErrorMessage(error)));
  }, [searchCursor, searchQuery, browsePageSize]);

  const exitSearch = useCallback(() => {
    setSearchActive(false);
    setSearchQuery("");
    setSearchResults({ kind: "loading" });
    setSearchCursor(null);
  }, []);

  const resolveCandidate = useCallback(
    (candidate: KnowledgeCandidate, action: "confirm" | "reject") => {
      const op = action === "confirm" ? confirmKnowledgeSource : rejectKnowledgeSource;
      op(candidate)
        .then(() => {
          setMessage(action === "confirm" ? "已确认 Vault。" : "已拒绝 Vault。");
          refreshDiscovery();
          refreshInventory();
        })
        .catch((error: unknown) => setMessage(readTesseraErrorMessage(error)));
    },
    [refreshDiscovery, refreshInventory],
  );

  const onPicker = useCallback(() => {
    setPickerBusy(true);
    requestVaultPicker()
      .then((result) => {
        setPickerBusy(false);
        if (result.payload.status === "cancelled") {
          setMessage("已取消选择。");
          return;
        }
        if (result.payload.status === "invalid") {
          setMessage("所选目录不可用或不是有效的 Vault。");
          return;
        }
        // selected: refresh discovery so the new candidate appears as a card.
        setMessage("已选择 Vault，请确认。");
        refreshDiscovery();
      })
      .catch((error: unknown) => {
        setPickerBusy(false);
        setMessage(readTesseraErrorMessage(error));
      });
  }, [refreshDiscovery]);

  const diagnosticText =
    discovery.kind === "ok" ? describeDiagnostic(discovery.value.diagnostic) : null;

  // Story 6.9 — Browse view (notes list for a single vault).
  if (browseVault) {
    return (
      <section aria-label={`浏览 ${browseVault.vaultName} 笔记`} id="tessera-obsidian" className="tsr-section">
        <h2 className="tsr-section__title">{browseVault.vaultName} · 笔记列表</h2>
        <p aria-live="polite" className="visually-hidden-text">{message}</p>
        <div className="tsr-rebuild">
          <button type="button" className="tsr-btn" onClick={exitBrowse}>
            ← 返回知识库清单
          </button>
        </div>
        {browseNotes.kind === "loading" ? (
          <p className="tsr-prose">正在加载笔记…</p>
        ) : null}
        {browseNotes.kind === "error" ? (
          <p role="alert" className="tsr-prose">{browseNotes.message}</p>
        ) : null}
        {browseNotes.kind === "ok" && browseNotes.value.length === 0 ? (
          <p className="tsr-prose">此知识库暂无可浏览的笔记。</p>
        ) : null}
        {browseNotes.kind === "ok" && browseNotes.value.length > 0 ? (
          <>
            <ul className="tsr-cards">
              {browseNotes.value.map((note) => (
                <li key={note.record_id} className="tsr-card">
                  <article className="tsr-card__body">
                    <div className="tsr-card__row">
                      <div className="tsr-card__main">
                        <div className="tsr-card__name">{note.vault_relative_path}</div>
                        <code className="tsr-card__path">{note.display_locator}</code>
                        <p className="tsr-card__meta">
                          {new Date(note.observed_at * 1000).toLocaleString("zh-CN")}
                        </p>
                        <p className="tsr-prose" style={{ whiteSpace: "pre-wrap", marginTop: "0.5rem" }}>
                          {note.excerpt}
                        </p>
                      </div>
                    </div>
                  </article>
                </li>
              ))}
            </ul>
            {browseCursor ? (
              <div className="tsr-rebuild">
                <button type="button" className="tsr-btn" onClick={loadMoreNotes}>
                  加载更多
                </button>
              </div>
            ) : null}
          </>
        ) : null}
      </section>
    );
  }

  // Story 6.9 — Search results view.
  if (searchActive) {
    return (
      <section aria-label="知识库搜索结果" id="tessera-obsidian" className="tsr-section">
        <h2 className="tsr-section__title">搜索「{searchQuery}」</h2>
        <p aria-live="polite" className="visually-hidden-text">{message}</p>
        <div className="tsr-rebuild">
          <button type="button" className="tsr-btn" onClick={exitSearch}>
            ← 返回知识库
          </button>
        </div>
        {searchResults.kind === "loading" ? <p className="tsr-prose">正在搜索…</p> : null}
        {searchResults.kind === "error" ? <p role="alert" className="tsr-prose">{searchResults.message}</p> : null}
        {searchResults.kind === "ok" && searchResults.value.length === 0 ? (
          <p className="tsr-prose">未找到匹配的笔记。</p>
        ) : null}
        {searchResults.kind === "ok" && searchResults.value.length > 0 ? (
          <>
            <ul className="tsr-cards">
              {searchResults.value.map((note) => (
                <li key={note.record_id} className="tsr-card">
                  <article className="tsr-card__body">
                    <div className="tsr-card__row">
                      <div className="tsr-card__main">
                        <h5 className="tsr-card__prov">{note.vault_name}</h5>
                        <div className="tsr-card__name">{note.vault_relative_path}</div>
                        <p className="tsr-card__meta">
                          {new Date(note.observed_at * 1000).toLocaleString("zh-CN")}
                        </p>
                        <p className="tsr-prose" style={{ whiteSpace: "pre-wrap", marginTop: "0.5rem" }}>
                          {note.excerpt}
                        </p>
                      </div>
                    </div>
                  </article>
                </li>
              ))}
            </ul>
            {searchCursor ? (
              <div className="tsr-rebuild">
                <button type="button" className="tsr-btn" onClick={loadMoreSearch}>加载更多</button>
              </div>
            ) : null}
          </>
        ) : null}
      </section>
    );
  }

  return (
    <section aria-label="Obsidian 知识库" id="tessera-obsidian" className="tsr-section">
      <h2 className="tsr-section__title">Obsidian 知识库</h2>
      <p aria-live="polite" className="visually-hidden-text">
        {message}
      </p>

      {/* Story 6.9 — cross-vault keyword search */}
      <section aria-label="搜索知识库" className="tsr-block">
        <h3 className="tsr-block__title">搜索笔记</h3>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            runSearch(searchQuery);
          }}
        >
          <div className="tsr-rebuild">
            <input
              type="text"
              className="tsr-input"
              placeholder="输入关键词搜索所有知识库…"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              aria-label="搜索关键词"
              style={{ marginRight: "0.5rem", padding: "0.4rem 0.6rem", minWidth: "300px" }}
            />
            <button type="submit" className="tsr-btn tsr-btn--primary">
              搜索
            </button>
          </div>
        </form>
      </section>

      {/* ---- Vault candidates (from registry + picker fallback) ---- */}
      <section aria-label="发现的 Vault" className="tsr-block">
        <h3 className="tsr-block__title">候选 Vault</h3>

        <div className="tsr-rebuild">
          <button
            type="button"
            className="tsr-btn tsr-btn--primary"
            onClick={onPicker}
            disabled={pickerBusy}
            aria-label="选择已有 Obsidian Vault"
          >
            {pickerBusy ? "正在打开选择器…" : "选择已有 Vault"}
          </button>
        </div>

        {discovery.kind === "loading" ? (
          <p className="tsr-prose">正在查找已注册的 Obsidian Vault…</p>
        ) : null}
        {discovery.kind === "error" ? (
          <p role="alert" className="tsr-prose">
            {discovery.message}
          </p>
        ) : null}
        {discovery.kind === "ok" && diagnosticText ? (
          <p role="status" className="tsr-prose">
            {diagnosticText}
          </p>
        ) : null}
        {discovery.kind === "ok" &&
        discovery.value.candidates.length === 0 &&
        !diagnosticText ? (
          <p className="tsr-prose">未发现已注册的 Obsidian Vault。可手动选择已有 Vault。</p>
        ) : null}
        {discovery.kind === "ok" && discovery.value.candidates.length > 0 ? (
          <ul className="tsr-candidates">
            {discovery.value.candidates.map((candidate) => (
              <li
                key={`${candidate.provider}:${candidate.root_path}`}
                className="tsr-candidate"
              >
                <div className="tsr-candidate__main">
                  <span className="tsr-candidate__provider">{candidate.provider}</span>
                  <span className="tsr-candidate__coverage">
                    {describeCoverage(candidate.coverage_level)}
                  </span>
                  <code className="tsr-candidate__path">{candidate.root_path}</code>
                </div>
                <div className="tsr-candidate__actions">
                  <button
                    type="button"
                    className="tsr-btn tsr-btn--primary"
                    onClick={() => resolveCandidate(candidate, "confirm")}
                  >
                    确认
                  </button>
                  <button
                    type="button"
                    className="tsr-btn"
                    onClick={() => resolveCandidate(candidate, "reject")}
                  >
                    拒绝
                  </button>
                </div>
              </li>
            ))}
          </ul>
        ) : null}
      </section>

      {/* ---- Knowledge Inventory (one card per confirmed Vault) ---- */}
      <section
        aria-label="知识库清单"
        aria-busy={inventory.kind === "loading"}
        className="tsr-block"
      >
        <h3 className="tsr-block__title">已确认的知识库</h3>
        {inventory.kind === "loading" ? (
          <p className="tsr-prose">正在加载知识库清单…</p>
        ) : null}
        {inventory.kind === "error" ? (
          <p role="alert" className="tsr-prose">
            {inventory.message}
          </p>
        ) : null}
        {inventory.kind === "ok" && inventory.value.length === 0 ? (
          <p className="tsr-prose">尚未确认任何 Obsidian Vault。</p>
        ) : null}
        {inventory.kind === "ok" && inventory.value.length > 0 ? (
          <ul className="tsr-cards">
            {inventory.value.map((item) => (
              <VaultCard
                key={item.source_id}
                item={item}
                onRefresh={refreshInventory}
                onBrowse={() => loadBrowse(item.source_id, item.vault_name)}
              />
            ))}
          </ul>
        ) : null}
      </section>
    </section>
  );
}

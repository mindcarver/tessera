/**
 * Tessera — `<Projects />` region: Tessera Project create / rename / delete +
 * explicit `(provider, native_project)` add-mapping / remove-mapping
 * (Story 5.1, UX-DR3 dev-stage).
 *
 * Architecture invariants honored here:
 * - **AD-1 application boundary:** every action goes through the loopback
 *   HTTP API via `src/api/projects.ts`; the UI never touches SQLite or the
 *   filesystem.
 * - **AD-24 explicit-only mapping:** `<Projects />` is the only place an
 *   association is formed. Creating a project creates zero mappings
 *   (`createProject` carries no mappings; the backend enforces it).
 * - **AD-27 cardinality:** a conflicting `addMapping` returns 409
 *   `mapping_conflict`; the safe message (allowlisted in
 *   `TESSERA_STABLE_ERROR_CODES`) renders verbatim via
 *   `readTesseraErrorMessage`. The UI never silently moves a mapping.
 * - **AD-21 / NFR-13 keyboard contract:** every action is keyboard-reachable.
 *   The destructive `delete` action uses the same inline-confirm region
 *   pattern Story 4.4 introduced for Rebuild (aria-expanded trigger, focus
 *   moved into the region on open, Esc / Cancel closes it). Status changes
 *   are announced via `aria-live`.
 * - **No router, no CSS framework:** plain controlled forms + the existing
 *   shared `readTesseraErrorMessage` helper, matching `Sources.tsx`'s shape.
 *
 * Mapping targets are `(provider, native_project)` pairs — the same native
 * identity already carried on Sources and canonical records — so the
 * add-mapping picker is fed by `getSourceInventory()`. The "Codex global"
 * branch surfaces as `(codex, null)`; Claude Code per-project branches
 * surface as `(claude_code, "<project key>")`.
 */

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactElement,
} from "react";
import {
  addMapping,
  createProject,
  deleteProject,
  listProjects,
  removeMapping,
  renameProject,
  type ProjectId,
  type TesseraProjectView,
} from "../../api/projects";
import { getSourceInventory, type SourceInventory } from "../../api/sources";
import { readTesseraErrorMessage } from "../../api/errors";
import { providerDisplayName } from "../../components/providerDisplayName";

type LoadState<T> =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ok"; value: T };

/**
 * One option in the add-mapping picker: a `(provider, native_project)` pair
 * the user can map to a project. Built client-side from the Source Inventory
 * (Story 5.1 keeps the backend projection-free; 5.2 fills the reserved
 * `tessera_project` filter slot).
 *
 * `label` is the keyboard-reachable visible string; `key` is the stable
 * `(provider, native_project)` identity used to dedupe the picker list.
 */
interface MappingOption {
  key: string;
  provider: string;
  native_project: string | null;
  label: string;
}

/**
 * Build the add-mapping picker options from the Source Inventory. Codex's
 * global store collapses to a single `(codex, null)` option (AD-27: at most
 * one active project per scope); Claude Code's per-project sources become
 * one option per distinct `native_project` key. Sources whose
 * `native_project` is null AND whose provider is not `codex` are skipped —
 * the backend rejects them as `bad_request` (only Codex's global store
 * carries null), so hiding them from the picker is honest.
 */
function buildMappingOptions(inventory: SourceInventory[]): MappingOption[] {
  const seen = new Set<string>();
  const options: MappingOption[] = [];
  for (const item of inventory) {
    if (item.native_project === null && item.provider !== "codex") continue;
    const key = `${item.provider}::${item.native_project ?? ""}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const provider = providerDisplayName(item.provider);
    const label =
      item.native_project === null
        ? `${provider} (global store)`
        : `${provider} — ${item.native_project}`;
    options.push({
      key,
      provider: item.provider,
      native_project: item.native_project,
      label,
    });
  }
  // Stable alphabetical ordering by `(provider, native_project)` so the
  // picker does not flicker between renders.
  return options.sort((a, b) => {
    if (a.provider !== b.provider) {
      return a.provider.localeCompare(b.provider, "en", { sensitivity: "base" });
    }
    return (a.native_project ?? "").localeCompare(b.native_project ?? "", "en", {
      sensitivity: "base",
    });
  });
}

/** Describe a mapping for screen-reader + visible render. */
function describeMapping(provider: string, nativeProject: string | null): string {
  if (nativeProject === null) {
    return `${providerDisplayName(provider)} (global store)`;
  }
  return `${providerDisplayName(provider)} — ${nativeProject}`;
}

export function Projects(): ReactElement {
  const [projects, setProjects] = useState<LoadState<TesseraProjectView[]>>({
    kind: "loading",
  });
  const [inventory, setInventory] = useState<LoadState<SourceInventory[]>>({
    kind: "loading",
  });
  const [statusMessage, setStatusMessage] = useState("");
  const [errorMessage, setErrorMessage] = useState("");

  // Create-form state.
  const [newName, setNewName] = useState("");

  // Per-project rename state (only one row is renamed at a time; mirroring
  // the inline-edit pattern in hand-rolled React UIs without router infra).
  const [renamingId, setRenamingId] = useState<ProjectId | null>(null);
  const [renameValue, setRenameValue] = useState("");

  // Per-project delete inline-confirm state. AD-21 contract: the destructive
  // action is a SEPARATE explicit activation; the region is keyboard-
  // reachable, focus moves in on open, Esc / Cancel closes it.
  const [deleteConfirmId, setDeleteConfirmId] = useState<ProjectId | null>(null);
  const deleteConfirmRef = useRef<HTMLDivElement>(null);

  // Per-project add-mapping picker state. One open picker at a time keeps
  // the keyboard flow simple; the picker is a `<select>` so it inherits the
  // standard keyboard-reachability of Sources / Search / Browse filters.
  const [addMappingForId, setAddMappingForId] = useState<ProjectId | null>(null);
  const [addMappingChoice, setAddMappingChoice] = useState<string>("");

  const refresh = useCallback(() => {
    listProjects()
      .then((result) => setProjects({ kind: "ok", value: result.payload }))
      .catch((error: unknown) => {
        setProjects({ kind: "error", message: readTesseraErrorMessage(error) });
      });
  }, []);

  useEffect(() => {
    refresh();
    getSourceInventory()
      .then((result) => setInventory({ kind: "ok", value: result.payload }))
      .catch((error: unknown) =>
        setInventory({ kind: "error", message: readTesseraErrorMessage(error) }),
      );
  }, [refresh]);

  const mappingOptions =
    inventory.kind === "ok" ? buildMappingOptions(inventory.value) : [];

  // --- Actions ---------------------------------------------------------------

  const onCreate = useCallback(() => {
    const trimmed = newName.trim();
    if (trimmed === "") {
      setErrorMessage("Project name cannot be empty.");
      return;
    }
    setErrorMessage("");
    setStatusMessage("Creating project…");
    createProject(trimmed)
      .then(() => {
        setNewName("");
        setStatusMessage("");
        refresh();
      })
      .catch((error: unknown) => {
        setStatusMessage("");
        setErrorMessage(readTesseraErrorMessage(error));
      });
  }, [newName, refresh]);

  const beginRename = useCallback((project: TesseraProjectView) => {
    setRenamingId(project.project_id);
    setRenameValue(project.name);
    setErrorMessage("");
  }, []);

  const submitRename = useCallback(
    (projectId: ProjectId) => {
      const trimmed = renameValue.trim();
      if (trimmed === "") {
        setErrorMessage("Project name cannot be empty.");
        return;
      }
      setErrorMessage("");
      setStatusMessage("Renaming project…");
      renameProject(projectId, trimmed)
        .then(() => {
          setRenamingId(null);
          setRenameValue("");
          setStatusMessage("");
          refresh();
        })
        .catch((error: unknown) => {
          setStatusMessage("");
          setErrorMessage(readTesseraErrorMessage(error));
        });
    },
    [renameValue, refresh],
  );

  const cancelRename = useCallback(() => {
    setRenamingId(null);
    setRenameValue("");
  }, []);

  const openDeleteConfirm = useCallback((projectId: ProjectId) => {
    setDeleteConfirmId(projectId);
    setErrorMessage("");
    // Move focus into the confirm region on the next tick (after render).
    window.setTimeout(() => deleteConfirmRef.current?.focus(), 0);
  }, []);

  const cancelDelete = useCallback(() => {
    setDeleteConfirmId(null);
    setErrorMessage("");
  }, []);

  const confirmDelete = useCallback(
    (projectId: ProjectId) => {
      setErrorMessage("");
      setStatusMessage("Deleting project…");
      deleteProject(projectId)
        .then((outcome) => {
          setDeleteConfirmId(null);
          setStatusMessage(
            `Deleted project with ${outcome.payload.removed_mappings} mapping${
              outcome.payload.removed_mappings === 1 ? "" : "s"
            }.`,
          );
          refresh();
        })
        .catch((error: unknown) => {
          setStatusMessage("");
          setErrorMessage(readTesseraErrorMessage(error));
        });
    },
    [refresh],
  );

  const onAddMapping = useCallback(
    (projectId: ProjectId) => {
      const choice = mappingOptions.find((o) => o.key === addMappingChoice);
      if (!choice) {
        setErrorMessage("Pick a native project to map first.");
        return;
      }
      setErrorMessage("");
      setStatusMessage("Adding mapping…");
      addMapping(projectId, choice.provider, choice.native_project)
        .then(() => {
          setAddMappingForId(null);
          setAddMappingChoice("");
          setStatusMessage("");
          refresh();
        })
        .catch((error: unknown) => {
          setStatusMessage("");
          setErrorMessage(readTesseraErrorMessage(error));
        });
    },
    [addMappingChoice, mappingOptions, refresh],
  );

  const onRemoveMapping = useCallback(
    (projectId: ProjectId, provider: string, nativeProject: string | null) => {
      setErrorMessage("");
      setStatusMessage("Removing mapping…");
      removeMapping(projectId, provider, nativeProject)
        .then(() => {
          setStatusMessage("");
          refresh();
        })
        .catch((error: unknown) => {
          setStatusMessage("");
          setErrorMessage(readTesseraErrorMessage(error));
        });
    },
    [refresh],
  );

  // Esc closes whichever inline region is open (delete confirm). Mirrors the
  // 4.4 rebuild-confirm Esc handler.
  useEffect(() => {
    if (!deleteConfirmId) return;
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setDeleteConfirmId(null);
        setErrorMessage("");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [deleteConfirmId]);

  // --- Render ----------------------------------------------------------------

  return (
    <section aria-label="Tessera projects">
      <h2>Projects</h2>
      <p
        aria-live="polite"
        data-testid="projects-status"
        className="visually-hidden-text"
      >
        {statusMessage}
      </p>
      {errorMessage ? (
        <p role="alert" data-testid="projects-error">
          {errorMessage}
        </p>
      ) : null}

      <section aria-label="Create a Tessera project">
        <h3>New project</h3>
        <label htmlFor="projects-new-name">Project name</label>
        {" "}
        <input
          id="projects-new-name"
          type="text"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e: KeyboardEvent<HTMLInputElement>) => {
            if (e.key === "Enter") {
              e.preventDefault();
              onCreate();
            }
          }}
          maxLength={128}
        />
        {" "}
        <button type="button" onClick={onCreate}>
          Create project
        </button>
      </section>

      <section
        aria-label="Tessera project list"
        aria-busy={projects.kind === "loading"}
      >
        <h3>Projects</h3>
        {projects.kind === "loading" ? <p>Loading projects…</p> : null}
        {projects.kind === "error" ? (
          <p role="alert">{projects.message}</p>
        ) : null}
        {projects.kind === "ok" && projects.value.length === 0 ? (
          <p data-testid="projects-empty">No Tessera projects yet.</p>
        ) : null}
        {projects.kind === "ok" && projects.value.length > 0 ? (
          <ul data-testid="projects-list">
            {projects.value.map((project) => (
              <li key={project.project_id} data-testid="projects-item">
                <article>
                  {/* Heading: name OR inline rename input. */}
                  {renamingId === project.project_id ? (
                    <>
                      <label htmlFor={`rename-${project.project_id}`}>
                        Project name
                      </label>
                      {" "}
                      <input
                        id={`rename-${project.project_id}`}
                        type="text"
                        value={renameValue}
                        onChange={(e) => setRenameValue(e.target.value)}
                        onKeyDown={(e: KeyboardEvent<HTMLInputElement>) => {
                          if (e.key === "Enter") {
                            e.preventDefault();
                            submitRename(project.project_id);
                          } else if (e.key === "Escape") {
                            e.preventDefault();
                            cancelRename();
                          }
                        }}
                        maxLength={128}
                      />
                      {" "}
                      <button
                        type="button"
                        onClick={() => submitRename(project.project_id)}
                      >
                        Save
                      </button>
                      {" "}
                      <button type="button" onClick={cancelRename}>
                        Cancel
                      </button>
                    </>
                  ) : (
                    <h4 data-testid="projects-item-name">{project.name}</h4>
                  )}

                  <dl>
                    <dt>Project id</dt>
                    <dd>
                      <code>{project.project_id}</code>
                    </dd>
                    <dt>Created</dt>
                    <dd>
                      {new Date(project.created_at * 1000).toLocaleString()}
                    </dd>
                    <dt>Updated</dt>
                    <dd>
                      {new Date(project.updated_at * 1000).toLocaleString()}
                    </dd>
                    <dt>Mapped native projects</dt>
                    <dd>
                      {project.mappings.length === 0 ? (
                        <span>No mappings yet.</span>
                      ) : (
                        <ul data-testid="projects-item-mappings">
                          {project.mappings.map((m) => (
                            <li
                              key={`${m.provider}::${m.native_project ?? ""}`}
                              data-provider={m.provider}
                            >
                              {describeMapping(m.provider, m.native_project)}
                              {" "}
                              <button
                                type="button"
                                onClick={() =>
                                  onRemoveMapping(
                                    project.project_id,
                                    m.provider,
                                    m.native_project,
                                  )
                                }
                              >
                                Remove mapping
                              </button>
                            </li>
                          ))}
                        </ul>
                      )}
                    </dd>
                  </dl>

                  {/* Row actions: rename / delete / add-mapping. */}
                  {renamingId !== project.project_id ? (
                    <button
                      type="button"
                      onClick={() => beginRename(project)}
                    >
                      Rename
                    </button>
                  ) : null}
                  {" "}
                  <button
                    type="button"
                    onClick={() => openDeleteConfirm(project.project_id)}
                    aria-expanded={deleteConfirmId === project.project_id}
                    aria-controls={`delete-confirm-${project.project_id}`}
                    disabled={deleteConfirmId === project.project_id}
                  >
                    Delete
                  </button>
                  {" "}
                  <button
                    type="button"
                    onClick={() => {
                      setAddMappingForId(project.project_id);
                      setAddMappingChoice("");
                    }}
                    aria-expanded={addMappingForId === project.project_id}
                    aria-controls={`add-mapping-${project.project_id}`}
                  >
                    Add mapping
                  </button>

                  {/* Inline delete-confirm region (AD-21 — keyboard-reachable,
                      focus moves in on open, Esc / Cancel closes it). */}
                  {deleteConfirmId === project.project_id ? (
                    <div
                      id={`delete-confirm-${project.project_id}`}
                      ref={deleteConfirmRef}
                      tabIndex={-1}
                      role="group"
                      aria-label={`Delete project ${project.name} confirmation`}
                    >
                      <p role="alert">
                        Delete this project and unmap its{" "}
                        {project.mappings.length === 1
                          ? "1 mapping"
                          : `${project.mappings.length} mappings`}?
                        Source files and other projects are not affected.
                      </p>
                      <button
                        type="button"
                        onClick={() => confirmDelete(project.project_id)}
                      >
                        Delete now
                      </button>
                      {" "}
                      <button type="button" onClick={cancelDelete}>
                        Cancel
                      </button>
                    </div>
                  ) : null}

                  {/* Inline add-mapping picker. Fed by getSourceInventory();
                      the same native identity already carried on Sources. */}
                  {addMappingForId === project.project_id ? (
                    <div
                      id={`add-mapping-${project.project_id}`}
                      role="group"
                      aria-label={`Add a mapping to project ${project.name}`}
                    >
                      <label htmlFor={`add-mapping-select-${project.project_id}`}>
                        Native project to map
                      </label>
                      {" "}
                      <select
                        id={`add-mapping-select-${project.project_id}`}
                        value={addMappingChoice}
                        onChange={(e) => setAddMappingChoice(e.target.value)}
                      >
                        <option value="">Pick a native project…</option>
                        {mappingOptions.map((opt) => (
                          <option key={opt.key} value={opt.key}>
                            {opt.label}
                          </option>
                        ))}
                      </select>
                      {" "}
                      <button
                        type="button"
                        onClick={() => onAddMapping(project.project_id)}
                      >
                        Add
                      </button>
                      {" "}
                      <button
                        type="button"
                        onClick={() => {
                          setAddMappingForId(null);
                          setAddMappingChoice("");
                        }}
                      >
                        Cancel
                      </button>
                      {mappingOptions.length === 0 ? (
                        <p>
                          No confirmed sources are available to map. Confirm a
                          source first.
                        </p>
                      ) : null}
                    </div>
                  ) : null}
                </article>
              </li>
            ))}
          </ul>
        ) : null}
      </section>
    </section>
  );
}

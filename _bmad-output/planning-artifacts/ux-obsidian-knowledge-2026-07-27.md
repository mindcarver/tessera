---
title: "Tessera Phase C.0 Obsidian Knowledge UX Contract"
status: final
date: 2026-07-27
scope: "Read-only multi-Vault onboarding, inventory, browse, search, provenance, and open"
binds:
  - UJ-5
  - FR-19..FR-25
  - NFR-13
  - NFR-14
  - AD-21
  - AD-37
  - AD-39
  - AD-40
---

# Tessera Phase C.0 Obsidian Knowledge UX Contract

## 1. Purpose and boundaries

This contract defines the user-facing behavior required for Phase C.0. It does
not add product scope beyond the approved PRD.

Tessera remains strictly read-only:

- users can discover, confirm, inventory, browse, keyword-search, filter, view
  Provenance, and ask Obsidian to open a note;
- users cannot create, edit, rename, move, delete, append, prepend, overwrite,
  retag, or otherwise mutate a note or Vault in Tessera;
- Agent Memory and Obsidian Knowledge remain separate top-level destinations;
- no screen presents a default mixed result set.

## 2. Information architecture

The primary navigation contains two explicit destinations:

1. **Agent Memory**
2. **Obsidian Knowledge**

The Obsidian Knowledge destination contains:

- **Sources** — discovery, confirmation, Knowledge Inventory, health, and
  rescan/recovery actions;
- **Browse** — Vault → folder → note drill-down without a query;
- **Search** — keyword query with Vault, folder-prefix, and modified-time
  filters.

Knowledge navigation uses:

```text
Obsidian Knowledge → Vault → Folder → Note
```

Agent-specific labels such as `Memory type`, `Native Project`, and `Tessera
Project` do not appear in Knowledge filters.

## 3. Vault onboarding

### 3.1 Discovered Vault cards

Each Candidate card displays:

- Vault display name;
- redacted/collapsible path presentation;
- discovery basis;
- provider `obsidian`;
- Coverage Level;
- confirmation state.

Card actions:

- **Confirm**
- **Reject**
- **Disable** for an already Confirmed Source

Confirmation and rejection operate on one Vault at a time.

### 3.2 Discovery states

| State | Required presentation | Available action |
| --- | --- | --- |
| Loading | Progress label; existing Agent Memory remains usable | Cancel only if the operation supports cancellation |
| Candidates found | One independently actionable card per Vault | Confirm or reject |
| No registered Vaults | Explicit “no registered Vault found” state | Choose an existing Vault |
| Registry missing | Diagnostic distinct from an empty registry | Choose an existing Vault |
| Registry corrupt/unsupported | Safe diagnostic without raw registry payload | Choose an existing Vault |
| Agent Memory startup continues | Knowledge diagnostic is source-scoped | Continue using Agent Memory |

### 3.3 Rust-owned native picker

The browser interaction is an action request, not a path submission:

```text
Browser: request_existing_vault_picker()
  → Rust core: open native OS directory dialog
  → Rust core: validate existing Obsidian Vault root
  → Rust core: return validated Candidate metadata or cancellation
```

The browser never receives or submits:

- a browser File System Access directory handle;
- an arbitrary filesystem token;
- a free-form path;
- a user-constructed URI.

Picker outcomes:

- **Valid existing Vault:** show a Candidate card; do not auto-confirm.
- **Cancelled:** restore focus to the picker button and preserve the page.
- **Not an Obsidian Vault:** show a safe validation error; do not persist a
  Source.
- **Outside policy boundary or unreadable:** show a safe error with no path
  leakage beyond the already user-visible selection.

### 3.4 Overlapping root resolution

When a Candidate overlaps a Confirmed Vault root:

- show both Vault names and redacted/collapsible roots;
- identify the relation as “contains” or “is contained by”;
- block confirmation of the second root;
- provide **Keep current Vault** and **Disable current Vault, then confirm the
  new Vault** actions;
- never trim, merge, or reassign ownership automatically;
- require a confirmation dialog before disabling the current Vault;
- restore focus to the conflict summary after cancellation.

Same-name Vaults with different non-overlapping roots remain independent.

## 4. Knowledge Inventory

Each Confirmed Vault card displays:

- Vault name and native Vault identity;
- source kind and provider;
- confirmed root presentation;
- Coverage Level;
- Source Health;
- complete supported Markdown count when coverage is `full`;
- last successful scan;
- current scan state;
- stale state;
- latest safe error.

Actions:

- **Browse**
- **Search this Vault**
- **Rescan**
- **Disable**

Truthful empty and failure states remain distinct:

- confirmed but never scanned;
- successful scan with zero supported notes;
- disabled;
- degraded with stale last-success data;
- error with no usable generation;
- scan in progress while the previous generation remains queryable.

## 5. Browse and Search

### 5.1 Browse

Browse supports:

- all Confirmed Vaults;
- one Vault;
- Vault-relative folder drill-down;
- note list with stable ordering and pagination.

Breadcrumb navigation preserves the current generation-bound scope. A stale
cursor returns the user to page one with an explanatory status message.

### 5.2 Search

Search defaults to every non-disabled Confirmed Obsidian Vault with a usable
current or stale last-success generation. Degraded Vaults with last-success data
remain in the default scope; the scope summary and every affected result are
visibly marked stale. A Confirmed Vault without a usable generation contributes
no records and remains visible as not indexed or unavailable. Disabled Vaults
are excluded until re-enabled.

Available filters:

- Vault/Source;
- Vault-relative folder prefix;
- absolute source-modified-time threshold.

The effective scope is always visible, including which Vaults contribute stale
data. Vault filters can narrow the scope or exclude a degraded Vault. Clearing
filters restores the same Knowledge-only default, not Agent Memory.

Empty states distinguish:

- no matching note;
- selected Vault not indexed;
- selected Vault unavailable;
- all selected Vaults disabled;
- stale results available from a previous successful generation.

## 6. Result and Provenance presentation

Each Knowledge result card displays:

- Knowledge domain;
- Vault name and Source;
- Vault-relative path;
- derived title and snippet, explicitly labeled as derived presentation;
- source modification time;
- observed time;
- Coverage Level;
- Source Health;
- stale state when applicable.

Tessera does not present inferred title, tag, property, backlink, or semantic
relationship as Obsidian-authored fact.

## 7. Open in Obsidian

The visible action is **Open in Obsidian**.

Interaction:

1. Browser sends only the trusted Knowledge `record_id`.
2. The action enters a busy state without removing the result card.
3. Rust resolves and validates the active record and dispatches the fixed
   `obsidian://open` action.
4. On OS dispatch acceptance, the UI reports **Open request sent to Obsidian**.
5. The UI never reports **Note opened successfully** based only on dispatch.

Error states distinguish:

- note missing;
- Vault moved or identity changed;
- target outside the Confirmed Vault;
- Obsidian URI handler unavailable;
- OS dispatch failure.

The error retains the result context and exposes a Source-status link. It does
not expose a raw URI or write-capable fallback.

## 8. Accessibility contract

All Phase C.0 flows require:

- logical heading hierarchy and landmarks;
- predictable semantic focus order;
- keyboard activation for every action;
- visible focus indicators;
- focus restoration after picker cancellation, dialogs, errors, and route
  transitions;
- live-region announcements for discovery, scan, filter, stale-cursor, open
  dispatch, and error status;
- status text/icons with accessible names; color is never the only signal;
- minimum WCAG AA text and interactive-component contrast;
- usable layout at 200% browser zoom and 320 CSS-pixel viewport width without
  loss of actions or horizontal page scrolling;
- reduced-motion behavior for loading and progress transitions;
- no automatic focus movement when background health or scan state changes.

The acceptance surface remains `tests/ui/accessibility.spec.ts`, extended with
Knowledge onboarding, Inventory, filters, and open status.

## 9. Loading and responsiveness

No fixed latency target is invented before real measurement.

The UI must nevertheless:

- show a loading state within the next render after an action;
- keep the last successful Inventory/query generation readable during scan;
- expose cancellation only where the core operation is cancellable;
- never represent a timeout as “no results”;
- use progressive status for discovery, scan, reconcile, and rebuild;
- keep open dispatch and visible-open evidence as separate states.

## 10. Acceptance evidence

Automated UI evidence covers:

- keyboard-only Vault onboarding;
- picker cancellation and error focus restoration;
- overlap-conflict actions;
- Inventory states;
- multi-Vault filter scope;
- stale-cursor recovery;
- open-dispatch status and safe failures;
- screen-reader status announcements.

Human evidence for correct visible opening is recorded separately in:

`_bmad-output/test-artifacts/obsidian-open-e2e.md`.

That artifact must not contain note bodies, private filenames, full Vault paths,
registry payloads, or raw `obsidian://` URIs.

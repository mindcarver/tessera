---
title: "Sprint Change Proposal — Read-only Multi-Vault Obsidian Knowledge Federation"
status: approved
date: 2026-07-26
workflow: bmad-correct-course
mode: batch
approval: approved
approved_by: Carver
approved_at: 2026-07-26
scope_classification: major
---

# Sprint Change Proposal — Read-only Multi-Vault Obsidian Knowledge Federation

## Decision Record

| Decision | Confirmed value |
| --- | --- |
| Change trigger | Carver keeps different knowledge domains in separate Obsidian Vaults and wants Tessera to manage them in one local interface. |
| Review mode | Batch |
| Trust boundary | Tessera remains read-only. |
| Included user actions | Inventory, browse, keyword search, filter, inspect provenance, and open the original note in Obsidian. |
| Excluded user actions | Create, edit, rename, move, delete, append to, prepend to, overwrite, or otherwise write a note or Vault. |
| Approval state | Approved by Carver with `C` on 2026-07-26. Formal planning-artifact and backlog updates are authorized; feature-code implementation remains outside this workflow. |

## 1. Issue Summary

### 1.1 Trigger

This is a new stakeholder requirement rather than a defect revealed by an
existing Story. Epic 1 through Epic 5 are currently marked `done`. Carver's
knowledge is distributed across multiple Obsidian Vaults, each representing a
different area of work. Tessera can currently inventory and query Agent Memory,
but it cannot inventory or query these user-owned Knowledge Sources.

The current PRD already names Obsidian as a future Phase C Knowledge Source, and
the Architecture Spine already reserves `source_kind=local_knowledge`. However,
the current product scope, implementation, and API only implement
`source_kind=agent_memory`.

### 1.2 Evidence

Read-only inspection on 2026-07-26 established the following local baseline:

| Evidence | Verified value |
| --- | ---: |
| Obsidian Vaults registered on this macOS machine | 6 |
| Registered Vault roots currently present | 6 |
| Markdown files outside `.obsidian/**` and `.git/**` | 1,813 |
| Total bytes in those Markdown files | 18,581,683 |
| Symbolic links observed in the six Vault roots | 0 |

No note body was read for this baseline; only Vault registry metadata, file
paths, file counts, file sizes, and link metadata were inspected.

Obsidian's official documentation confirms that:

- a Vault is a local folder whose notes are Markdown files, and multiple folders
  may be opened as separate Vaults;
- `.obsidian` stores Vault-specific configuration and workspace state;
- `obsidian://open` can open a note by Vault ID and Vault-relative file path.

References:

- [How Obsidian stores data](https://obsidian.md/help/Files%2Band%2Bfolders/How%2BObsidian%2Bstores%2Bdata)
- [Manage vaults](https://obsidian.md/help/Files%2Band%2Bfolders/Manage%2Bvaults)
- [Obsidian URI](https://obsidian.md/help/Extending%2BObsidian/Obsidian%2BURI)

### 1.3 Core problem statement

Carver cannot use Tessera to answer:

> "Which Vault contains this knowledge, what matching notes exist across all
> confirmed Vaults, how fresh and complete are those results, and can I open the
> exact original note in Obsidian?"

The solution must preserve each Markdown file in place, preserve each Vault as
an independent Source, provide truthful health and provenance, and never turn
Obsidian notes into Agent Memory records.

### 1.4 Product boundary selected by this proposal

Phase C.0 is a local, read-only Obsidian Knowledge Source expansion:

- each confirmed Vault is one independent `local_knowledge` Source;
- the initial Knowledge scope searches and browses all confirmed Obsidian
  Vaults, or a selected Vault;
- Agent Memory and Obsidian Knowledge remain separate top-level domains in the
  UI and data model;
- a default mixed "Agent Memory + Obsidian" result set is deferred. If later
  requested, it must use an explicit federated read projection required by
  AD-19, not a shared write model;
- the initial artifact set is Markdown notes only;
- first-release filters are Vault, Vault-relative folder, and source
  modification time;
- tags, properties, backlinks, embeds, Canvas, attachments, semantic search,
  summaries, and knowledge graphs are not part of Phase C.0.

## 2. Impact Analysis

### 2.1 Epic impact

| Epic | Impact |
| --- | --- |
| Epic 1 — Codex foundation | Remains `done`. Reuse confirmation, path policy, generation/fencing, atomic visibility, and open-by-record safety patterns. Do not reopen its Stories. |
| Epic 2 — Cross-Agent federation | Remains `done`. Reuse inventory and query interaction patterns, but do not treat Obsidian as a third Agent Provider. |
| Epic 3 — Browse and visualization | Remains `done`. Reuse pagination, EmptyState, keyboard, breadcrumb, and result-card presentation patterns. Knowledge DTOs and filters remain separate. |
| Epic 4 — Health and recovery | Remains `done`. Reuse source-scoped health, stale last-success, watcher-as-hint, rebind, and rebuild concepts. Vault event filtering and cadence require new acceptance criteria. |
| Epic 5 — Tessera Projects | Remains `done` and is not a Phase C.0 dependency. A Vault is not a `NativeProject`, and no automatic Tessera Project mapping is introduced. |
| **New Epic 6** | **Add "Read-only Obsidian Multi-Vault Knowledge Federation" with six new Stories.** |

No current Epic becomes obsolete. No completed Story needs rollback. The new
Epic follows Epic 5 as a Phase C.0 expansion.

### 2.2 Artifact conflicts and required changes

| Artifact | Current conflict | Required change after approval |
| --- | --- | --- |
| PRD | Obsidian is Phase C and explicitly outside MVP/non-goals. | Add Phase C.0, UJ-5, terminology, FR-19..FR-25, Knowledge Source scope, success signals, risks, and read-only non-goals. |
| PRD Addendum | Obsidian is only a future-direction paragraph. | Add the Phase C.0 Obsidian Supported Artifact Matrix, discovery boundary, open semantics, and known limitations. |
| Architecture Spine | AD-10/AD-19 reserve Knowledge Source but do not define its implemented schema, identity, query, or open target. | Activate `local_knowledge`; add separate schema/identity/parser/migration/query rules and Obsidian-specific discovery/open rules. |
| SPEC | CAP-1..CAP-11 only cover Agent Memory; Obsidian is a non-goal. | Add Knowledge capabilities and constraints without weakening existing Agent Memory capabilities. |
| Requirements Matrix | FR-1..FR-18 and artifact matrix are Agent-Memory-only. | Add FR-19..FR-25, Obsidian artifact matrix, and Knowledge terminology. |
| Epics | Only Epics 1..5 exist. | Add Epic 6, six Stories, FR coverage, UX decisions, and dependencies. |
| UX | No standalone UX specification exists; current UX decisions are embedded in Stories. | Continue that convention and add UX-DR9..UX-DR14 to `epics.md`. |
| Sprint status | Epics 1..5 are `done`; no Phase C backlog exists. | Only after approval, add Epic 6 and Stories 6.1..6.6 as `backlog`; preserve every existing status. |
| Test/benchmark artifacts | Existing fixtures and performance baseline are Agent-Memory-sized. | Add Obsidian fixtures, security matrix, real multi-Vault acceptance, and a new measured baseline. |

### 2.3 Current implementation gap

Repository inspection shows this is not a one-line Adapter registration:

| Current implementation | Required Phase C.0 behavior |
| --- | --- |
| `SourceKind` only accepts `AgentMemory`. | Add `LocalKnowledge` and reject unknown persisted values instead of silently treating them as Agent Memory. |
| Confirm/discover dispatch only knows `codex` and `claude_code` and hard-codes `AgentMemory`. | Dispatch by `(source_kind, connector)` and confirm each Vault as `LocalKnowledge`. |
| `ProviderAdapter`, `ProviderMemoryType`, and `CandidateSource` are Agent-Memory-specific. | Add a separate Knowledge Connector contract; do not widen Agent terminology until it becomes ambiguous. |
| All canonical rows live in `memory_records` with `rec_` identity. | Add an additive `knowledge_records` migration with a distinct `krec_` identity namespace and Vault metadata extension. |
| Search and Browse read only `memory_records` and filter by provider/memory type/native project. | Add Knowledge Query contracts with Vault/folder/modified-time filters. Do not implicitly mix domains. |
| `SourceInventory` and frontend guards assume `agent_memory`. | Add Knowledge Inventory DTOs and top-level domain presentation. |
| Open-original passes a file path to the system default application. | Validate a trusted Knowledge record, then construct only an encoded `obsidian://open` URI using Vault ID + relative note path. |
| Rebuild scans every confirmed Source through the Agent adapter registry. | Dispatch rebuild by source kind and keep Agent/Knowledge record deletion and rebuild boundaries independent. |
| Watchers turn any root event into a dirty hint, with a 60-second forced reconcile. | Ignore `.obsidian/**`, attachments, and all out-of-matrix events before scheduling; measure a bounded Vault self-heal cadence instead of inheriting the Agent cadence. |
| Keyword search uses SQLite `instr`, not FTS5. | Measure the real 1,813-note baseline. Keep literal search only if it passes the new gate; otherwise adopt a Knowledge-specific FTS strategy without changing Agent Memory semantics. |

### 2.4 Unchanged invariants

- Rust core remains the only filesystem and application boundary.
- The HTTP service remains loopback-only with no outbound network dependency.
- Only Confirmed Sources are readable.
- Tessera writes only its own app-data/index.
- Source content is treated as untrusted text.
- Watcher events remain hints; reconcile remains truth.
- Failed scans never replace the previous successful generation.
- Errors remain source-scoped and redacted.
- No cloud, account, remote sync, writeback, AI summary, vector search, or
  conflict resolution is added.

## 3. Change Analysis Checklist

### 3.1 Understand the trigger and context

| Item | Status | Finding |
| --- | --- | --- |
| 1.1 Triggering Story | `[N/A]` | New stakeholder requirement after Epic 1..5; no defect Story. |
| 1.2 Core problem | `[x]` | New requirement / planned Phase C activation. |
| 1.3 Evidence | `[x]` | Six registered existing Vaults; 1,813 Markdown files; PRD/Addendum/Architecture already reserve Obsidian as Knowledge Source. |

### 3.2 Epic impact

| Item | Status | Finding |
| --- | --- | --- |
| 2.1 Current Epic viability | `[x]` | Existing Epics remain valid and complete. |
| 2.2 Epic-level change | `[x]` | Add Epic 6; do not modify completion status of Epics 1..5. |
| 2.3 Remaining/future Epic review | `[x]` | Epic 1/3/4 patterns are dependencies; Epic 5 is not required. |
| 2.4 New/obsolete Epics | `[x]` | One new Epic is required; none become obsolete. |
| 2.5 Order/priority | `[x]` | Epic 6 is the next product Epic; its internal order is defined below. |

### 3.3 Artifact conflict analysis

| Item | Status | Finding |
| --- | --- | --- |
| 3.1 PRD conflict | `[x]` | Phase C is planned, but Obsidian is explicitly excluded from current scope. |
| 3.2 Architecture conflict | `[x]` | Extension point exists, but separate Knowledge schema/query/open rules are not yet defined or implemented. |
| 3.3 UX conflict | `[x]` | No standalone UX artifact; existing Agent-only labels, filters, and drill-down cannot represent Vault/folder semantics honestly. |
| 3.4 Other artifacts | `[x]` | SPEC, requirements matrix, sprint status, fixtures, benchmarks, security tests, and user documentation require updates. |

### 3.4 Path-forward evaluation

| Option | Viability | Effort | Risk | Assessment |
| --- | --- | --- | --- | --- |
| Option 1 — Direct adjustment | **Viable / selected** | High | Medium | Add a bounded Phase C.0 Epic while retaining all existing work. |
| Option 2 — Rollback | Not viable | High disruption | High | Rolling back completed Agent capabilities provides no simplification or user value. |
| Option 3 — Redefine the original MVP | Not selected | Medium | Medium | Phase A remains the proven baseline. Obsidian is a new phase, not a retroactive change to Phase A acceptance. |

**Recommended path:** Option 1, implemented as an additive Phase C.0 Epic with
an architecture gate before Story 6.1.

## 4. Recommended Approach

### 4.1 Approach

Add **Epic 6 — Read-only Obsidian Multi-Vault Knowledge Federation**.

Preserve the existing Source Registry and operational safety mechanisms, but
introduce an independent `local_knowledge` vertical slice:

```text
Shared operational substrate
├── Source Registry / confirmation / health
├── path policy / fingerprint / rebind
├── scan run ownership / generation / fencing
├── watcher hint / reconcile / stale last-success
└── loopback HTTP / accessibility patterns

Agent Memory domain                     Obsidian Knowledge domain
├── ProviderAdapter                     ├── KnowledgeConnector
├── memory_records / rec_*              ├── knowledge_records / krec_*
├── ProviderMemoryType filters          ├── Vault/folder/mtime filters
├── Agent Search/Browse DTO             ├── Knowledge Search/Browse DTO
└── system file opener                  └── validated obsidian://open
```

The first release does not add an "All domains" query. This keeps the change
focused on unifying Obsidian Vaults, avoids falsely equating a user-authored
note with Agent Memory, and follows AD-19. A later cross-domain search must be
an explicit, neutral, read-only projection.

### 4.2 Relative effort and timeline impact

Overall effort is **High** because the change adds a new domain model, schema,
connector, query surface, UI domain, and real-data acceptance gate.

Overall technical risk is **Medium** because the current architecture already
reserves the boundary and the safety substrate is reusable.

No calendar estimate is asserted: current team velocity and capacity were not
provided. The verifiable schedule impact is one new Epic with this critical
path:

```text
Architecture approval
  → 6.1 Domain + discovery
  → 6.2 Read-only indexing
  → 6.3 Inventory/reconcile ─┐
  → 6.4 Browse/search ───────┼→ 6.5 Provenance/open → 6.6 Real acceptance gate
                             ┘
```

### 4.3 Scope classification

**Major** under the Correct Course workflow:

- it activates a new product phase;
- it adds a new domain type and persistence/query model;
- it changes PRD, architecture, SPEC, Epic, UX, and validation artifacts.

It is nevertheless an additive major change: no rollback or fundamental
replacement of the existing engine is recommended.

### 4.4 Main risks and mitigations

| Risk | Consequence | Required mitigation |
| --- | --- | --- |
| Obsidian's local Vault registry JSON is not a documented stable API. | Discovery silently returns zero Vaults after an Obsidian format change. | Treat registry parsing as best-effort with fixtures and visible diagnostics; provide an OS folder-picker fallback constrained to an existing Vault root; never scan all of HOME. |
| Agent and Knowledge records are conflated. | Misleading filters, identity collisions, unsafe migrations. | Separate connector, `knowledge_records`, `krec_`, parser registry, query DTO, and migration history. |
| Existing unknown `source_kind` values fall back to Agent Memory. | Corrupt/migrated Knowledge Sources are misclassified. | Change registry decode to fail safely and surface corruption; never coerce unknown kinds. |
| Source fingerprint v1 omits source kind. | Changing the old algorithm could drift existing Source IDs. | Preserve Agent fingerprints; define a compatible Knowledge fingerprint rule/version and test upgrade identity. |
| `.obsidian/workspace.json` changes when a note opens. | Open action produces a watcher/reconcile loop. | Filter `.obsidian/**` before dirty-hint scheduling and exclude it from the artifact manifest. |
| Six Vaults are periodically fully rescanned. | I/O churn and poor battery/RSS behavior. | Filter events, coalesce bursts, use a lightweight no-op manifest check, and set cadence only after real measurement. |
| Current scanner reads each file without a per-file byte bound. | A very large note can cause memory pressure. | Define a measured, documented maximum note size before Story 6.2 implementation; oversized notes receive safe diagnostics without losing the previous generation. |
| Nested or overlapping Vault roots. | Duplicate notes and ambiguous provenance. | Detect overlapping confirmed roots; block simultaneous confirmation until the user chooses the intended ownership boundary. |
| Symlink traversal or retargeting. | Read outside a confirmed Vault. | Do not recurse symlink directories; realpath-check every file and revalidate at read/open time; preserve TOCTOU tests. |
| Obsidian URI encoding or action injection. | Wrong note opens or a write-capable URI is invoked. | Build the URI server-side from trusted fields; fixed action `open`; allow only Vault ID and relative file parameters; percent-encode reserved characters. |
| `open::that` reports dispatch success but Obsidian opens the wrong target. | False success. | Add real Obsidian human E2E covering correct Vault/note; distinguish "URI dispatched" from "verified visible note" in evidence. |
| `instr` search does not scale to real Vaults. | Slow multi-Vault search. | Benchmark current literal search first; introduce Knowledge-specific FTS only if the measured gate fails. |
| Obsidian or cloud-sync software mutates files concurrently. | Scan drift or false zero-write attribution. | Keep atomic manifests; reject dirty generations; separate Tessera-caused writes from observed third-party changes in acceptance evidence. |

## 5. Detailed Change Proposals

### 5.1 PRD changes

#### PRD-P1 — Activate Phase C.0

**Section:** `1.1 当前阶段`

**OLD**

> Phase C — 多知识源联邦：连接 Obsidian、RAGFlow、飞书知识库等，但
> Agent Memory 与 Knowledge Source 保持独立领域类型。

**NEW**

> **Phase C.0 — 本机 Obsidian 多 Vault 知识联邦：** 在保留 Phase A
> Agent Memory 能力的基础上，接入本机已注册或由用户显式选择的 Obsidian
> Vault。每个 Vault 是独立的 `local_knowledge` Source；Tessera 只读清点、
> 索引、浏览、关键词搜索、筛选、展示 Provenance，并调用 Obsidian 打开
> 原文。Agent Memory 与 Knowledge Source 保持独立领域类型，默认不混合
> 查询。
>
> **Phase C.1+ — 其他知识源：** RAGFlow、飞书知识库及显式跨领域查询在
> Phase C.0 真实使用验证后单独评估。

**Rationale:** Activates the already planned phase without retroactively
changing Phase A acceptance.

#### PRD-P2 — Add the user job and UJ-5

**Section:** `2.2 Jobs To Be Done` and `2.4 关键用户旅程`

**OLD**

> No job or journey covers multiple user-authored Vaults.

**NEW**

> - 当我的不同知识领域分散在多个 Obsidian Vault 时，我想在一个本机界面
>   统一清点、浏览和搜索，并按 Vault、文件夹和修改时间筛选，以便不必逐个
>   切换 Vault 查找。
>
> - **UJ-5. Carver 统一浏览和搜索多个 Obsidian Vault。**
>   - **背景：** Carver 使用多个 Vault 管理不同知识领域。
>   - **进入状态：** Obsidian 已在本机注册 Vault，或用户通过受限目录选择器
>     选择一个现有 Vault。
>   - **路径：** Tessera 发现 Vault Candidate → Carver 逐个确认 → 建立只读
>     Knowledge Index → Knowledge Inventory 展示范围、数量、健康和最近扫描
>     → 跨全部或指定 Vault 浏览/搜索 → 按 Vault/文件夹/修改时间筛选 →
>     查看 Provenance → 用 Obsidian 打开正确原文。
>   - **价值时刻：** Carver 在不逐个切换 Vault 的情况下找到目标知识，并能
>     核验正确 Vault 和原始 Markdown 文件。
>   - **结果：** Vault 和笔记不被 Tessera 修改；确认关系和派生索引在重启后
>     保留。

#### PRD-P3 — Add Knowledge terminology

**Section:** `3. 术语表`

**NEW terms**

- **Local Knowledge Source** — a user-confirmed local knowledge root whose
  authored files remain the fact source.
- **Obsidian Vault** — one Obsidian-registered local folder, represented as one
  independent `local_knowledge` Source.
- **Knowledge Record** — a Tessera-owned, rebuildable representation of one
  supported Vault note; it is not an Agent Memory record.
- **Vault-relative Path** — the note path relative to the confirmed Vault root,
  used for identity, filtering, provenance, and Obsidian open.
- **Knowledge Index** — the deletable/rebuildable derived index built from
  Confirmed Knowledge Sources.

#### PRD-P4 — Add FR-19 through FR-25

**FR-19 — Discover and confirm multiple Obsidian Vaults**

- Read Vault registry metadata without reading note bodies.
- Produce one Candidate per existing registered Vault with provider, Vault
  identity, path, discovery basis, and coverage.
- A registry parse failure must be visible; it must not look identical to
  "Obsidian has no Vaults".
- Provide a native directory-picker fallback restricted to an existing
  Obsidian Vault; do not provide a free-form arbitrary path input and do not
  recursively search HOME.
- Only a Confirmed Vault may be scanned.

**FR-20 — View Knowledge Source Inventory**

- Display source kind, provider, Vault name, path, coverage, health, last
  successful scan, complete Markdown-note count when coverage is `full`, stale
  state, and latest safe error.
- Keep same-name Vaults distinct by native Vault identity and confirmed root.
- Exclude `.obsidian/**` and other non-note artifacts from the note count.

**FR-21 — Build a read-only Knowledge Index**

- Recursively index supported regular Markdown notes inside a Confirmed Vault.
- Keep Knowledge records in a separate identity/table/parser namespace.
- Initial Knowledge Record granularity is one Markdown file per record.
- A note keeps stable identity while its confirmed Source and normalized
  Vault-relative path remain unchanged; rename/move creates a new locator.
- A scan, failed scan, cancellation, retry, or rebuild must not change any
  Vault file, byte, size, or mtime.
- A failed or drifting scan must not replace the previous successful generation.

**FR-22 — Browse, search, and filter across confirmed Vaults**

- Browse or keyword-search all Confirmed Obsidian Vaults or a selected Vault.
- Filter by Vault, Vault-relative folder prefix, and source modification time.
- Display the effective scope and distinguish no match, not indexed, and
  unavailable.
- Do not implicitly include Agent Memory in the Knowledge result set.

**FR-23 — Display Knowledge Provenance**

- Every result includes source kind, provider, Source ID, native Vault ID/name,
  Vault-relative path, source modification time, observed time, coverage, and
  health.
- Display titles/snippets as derived presentation; do not represent inferred
  metadata as an Obsidian-authored fact.
- Do not automatically summarize, merge, deduplicate, or resolve conflicts.

**FR-24 — Open the original note in Obsidian**

- Browser submits only a trusted Knowledge `record_id`.
- Rust core resolves the active record, verifies the current target remains
  inside the Confirmed Vault, and builds `obsidian://open` with encoded Vault ID
  and relative file path.
- No write-capable URI action or parameter (`new`, `append`, `prepend`,
  `overwrite`, content) may be constructed.
- Missing note, moved Vault, unregistered URI handler, or dispatch failure
  returns a safe error without reporting false success.

**FR-25 — Reconcile, isolate failures, and rebuild Knowledge Index**

- Watcher events are hints; only supported Markdown changes may schedule a
  Vault reconcile.
- `.obsidian/**`, `.git/**`, trash, attachments, Canvas, plugin files, and
  other excluded paths do not schedule a scan.
- One Vault failure never blocks other Vaults or Agent Memory.
- Preserve and mark the previous successful Vault generation as stale.
- Rebuild Knowledge records without deleting Agent records, confirmed Sources,
  or Tessera Project mappings.

#### PRD-P5 — Update scope, NFRs, success metrics, risks, and non-goals

**Remove from current exclusions**

> Obsidian Connector is entirely out of scope.

**Replace with**

> Obsidian is in Phase C.0 only as a local, read-only Markdown Knowledge Source.
> RAGFlow and Feishu remain out of scope.

**Broaden NFRs without weakening Agent boundaries**

- NFR-1/2/3/5/6/7/8/9/10/11/12/13 apply to both Agent Memory and Knowledge
  Source with domain-specific schemas.
- Add **NFR-14 — Vault zero-write:** Tessera's scan, search, browse, filter,
  health, and rebuild paths never write inside a Vault. The user-triggered
  open action delegates to Obsidian; Obsidian-owned `.obsidian` workspace
  changes are not indexed and must not schedule Tessera reconcile.
- Add a bounded note-size safety policy before implementation; choose the bound
  from real data and security requirements, not guesswork.

**Add success metrics**

- **SM-8 — Multi-Vault closure:** discover the six currently registered Vaults,
  confirm a selected set, browse/search across them, filter to the expected
  Vault/folder/time scope, and open predetermined notes in the correct Vault.
- **SM-9 — Zero Vault mutation:** successful/failed/cancelled scan and rebuild
  do not change Vault paths, bytes, sizes, or mtimes in controlled fixtures;
  real acceptance distinguishes external Obsidian/Sync changes.
- **SM-10 — Knowledge provenance:** every displayed result resolves to the
  correct confirmed Vault and relative Markdown path.
- **SM-11 — Vault failure isolation:** making one Vault missing or unreadable
  leaves all other Vaults and Agent Memory usable and truthfully marks stale
  results.
- **SM-12 — Measured scale:** record cold scan, no-op reconcile, incremental
  update, query latency, RSS, file descriptors, threads, and index size on the
  real 1,813-note dataset before locking thresholds.

**Explicit Phase C.0 non-goals**

- creating, editing, renaming, moving, deleting, appending to, prepending to, or
  overwriting any note or Vault;
- modifying frontmatter, properties, tags, links, or `.obsidian` configuration;
- Obsidian Sync integration, cloud upload, team collaboration, or permissions;
- attachments, OCR/PDF extraction, Canvas, Bases, plugin databases, trash;
- tag/property/backlink/embed graph semantics;
- AI question answering, embeddings, semantic retrieval, summaries,
  deduplication, or conflict resolution;
- RAGFlow, Feishu, arbitrary directory ingestion, or whole-HOME discovery;
- default mixed Agent Memory + Knowledge search.

### 5.2 PRD Addendum changes

#### ADD-P1 — Replace "future Obsidian direction" with Phase C.0 contract

**OLD**

> Prefer a local read-only Vault Connector. Markdown files are the fact source;
> Obsidian CLI may be an enhancement but is not a hard dependency.

**NEW**

Keep that rule and add:

| Obsidian Phase C.0 | Included | Excluded |
| --- | --- | --- |
| Discovery | Registered Vault metadata from the OS-specific Obsidian system folder; constrained existing-Vault folder picker fallback | Whole-HOME scan, free-form path input, remote Vault APIs |
| Content | Regular Markdown notes under non-hidden Vault-relative paths | `.obsidian/**`, all dot paths, `.git/**`, trash, `.canvas`, attachments, binary files, plugin data |
| Record grain | One Markdown file = one Knowledge Record | Heading/block identity, backlinks, graph edges, structured property/tag semantics |
| Query facets | Vault, folder prefix, modified time | AI/semantic, tag/property/backlink filters |
| Open | Encoded `obsidian://open` using trusted Vault ID + relative note path | `new`, `append`, `prepend`, `overwrite`, content-bearing URI parameters |

The local registry format is an observed integration surface, not a documented
Obsidian API. Its parser must be fixture-protected and fail visibly. Obsidian
CLI remains optional and is not needed for indexing or opening.

### 5.3 Architecture changes

#### ARCH-P1 — Revise AD-10

**OLD**

> MVP implements only `agent_memory`; Knowledge Source is future work.

**NEW**

> Phase A implements `agent_memory`; Phase C.0 implements
> `local_knowledge/obsidian`. Both use the Source Registry and operational
> substrate, but they retain independent connector, record, parser, migration,
> query-filter, and open-target contracts. `remote_knowledge` remains deferred.

#### ARCH-P2 — Revise AD-19

**OLD**

> Future Knowledge Source schemas cannot alias Agent Memory.

**NEW**

> Knowledge Source schemas do not alias Agent Memory. `knowledge_records` use a
> distinct `krec_` namespace and migration history; `memory_records` and
> `ProviderMemoryType` remain Agent-only. Any future cross-domain query uses an
> explicit neutral read projection carrying `source_kind` and domain-specific
> provenance. Phase C.0 does not enable that mixed projection by default.

#### ARCH-P3 — Add AD-37 through AD-40

**AD-37 — Registered Vault discovery is metadata-only and honest**

- The Obsidian connector reads only OS-specific Vault registry metadata during
  discovery.
- Each existing registered Vault becomes one Candidate.
- Missing/corrupt/unsupported registry formats produce a visible discovery
  diagnostic.
- A native directory picker may select an existing Vault root as a fallback;
  no free-form path or HOME recursion is allowed.
- Confirmation canonicalizes the root and stores provider-native Vault ID as
  Knowledge source metadata.

**AD-38 — Knowledge record identity and storage are independent**

- Add `knowledge_records` and Knowledge parser registry.
- Use `krec_` identity derived from `source_id + vault-relative-path +
  unit_kind=note`.
- The v1 record grain is file-level.
- Vault rename/move/rebind follows Source identity rules; note rename/move
  changes locator/identity rather than fuzzy-merging.
- Preserve Agent fingerprint behavior; do not rewrite existing Source IDs.
- Unknown persisted `source_kind` is corruption, never Agent-Memory fallback.

**AD-39 — Knowledge query is a separate bounded contract**

- Knowledge Search/Browse has its own DTO and cursor binding for Source,
  Vault-relative folder, modified-time filter, active generation, and policy
  revision.
- Results carry `source_kind`, Knowledge record kind, Vault identity, relative
  path, source revision, coverage, and health.
- The UI presents Agent Memory and Obsidian Knowledge as explicit domains.
- A future combined query must be a neutral read projection, never a shared
  canonical write table.

**AD-40 — Open-in-Obsidian is a non-mutating, fixed-action capability**

- UI sends only `krec_` record ID.
- Core resolves active record and revalidates current containment.
- Core constructs only `obsidian://open?vault=<id>&file=<relative-path>`.
- All dynamic fields are percent-encoded.
- Writing actions/parameters are impossible by type/constructor.
- Successful URI dispatch is not evidence that the target became visibly open;
  real Obsidian E2E is a separate acceptance item.

#### ARCH-P4 — Update structural and data model

Add:

```text
server/src/
  domain/knowledge/                 # Knowledge record, Vault metadata, filters
  domain/ports/knowledge_connector.rs
  application/knowledge/            # discover, confirm, scan, query, open
  adapters/obsidian.rs
  index/knowledge_store.rs

src/features/knowledge/
  Sources.tsx
  Browse.tsx
  Search.tsx
```

Additive persistence:

```text
SOURCE (shared registry)
  └── OBSIDIAN_VAULT_METADATA (source_id, vault_id, display_name)
      └── KNOWLEDGE_RECORD (krec_*, generation, relative_path, title, body,
                            modified_at, source_revision, parser_version)
```

`scan_runs` and generation ownership may be reused only after dispatch and
reset/rebuild behavior are made source-kind-aware. `knowledge_records` must not
be inserted into `memory_records`.

### 5.4 SPEC and requirements-matrix changes

**SPEC**

- Preserve CAP-1..CAP-11 as Agent Memory capabilities.
- Add CAP-12..CAP-18 corresponding to FR-19..FR-25.
- Change the Obsidian non-goal to the bounded Phase C.0 contract.
- Add the separate-domain, zero-write, discovery-fallback, Vault identity,
  event-filter, open-action, and performance constraints.

**Requirements Matrix**

- Add FR-19..FR-25 rows.
- Add NFR-14.
- Add the Obsidian Phase C.0 artifact matrix.
- Define `Local Knowledge Source`, `Obsidian Vault`, `Knowledge Record`,
  `Knowledge Index`, and Vault-relative path.
- Move RAGFlow/Feishu and cross-domain combined search to future direction.

### 5.5 Epic and Story changes

#### New Epic 6 — Read-only Obsidian Multi-Vault Knowledge Federation

Carver can confirm multiple local Obsidian Vaults, view their coverage and
health, build a read-only Knowledge Index, browse and keyword-search across all
confirmed Vaults, filter by Vault/folder/modified time, inspect exact
provenance, and open the original note in the correct Obsidian Vault.

**FR coverage:** FR-19..FR-25
**Dependencies:** Epic 1 path/source/generation foundation; Epic 3 interaction
patterns; Epic 4 health/reconcile/rebuild patterns. Epic 5 is not required.

##### Story 6.1 — Discover and confirm Obsidian Vault Knowledge Sources

**As Carver,** I want Tessera to discover my registered Obsidian Vaults and let
me confirm them independently, **so that** only the Vaults I choose become
readable Knowledge Sources.

**Acceptance criteria**

- Existing registered Vaults appear as deterministic Candidates with provider,
  Vault identity/name, path, basis, and `full` coverage; discovery reads no
  note body.
- Missing/corrupt/unknown registry shapes produce a safe visible diagnostic
  and do not block Agent Memory startup.
- A constrained existing-Vault folder picker is available when registry
  discovery is unavailable; no free-form path or HOME scan exists.
- Confirmation persists `source_kind=local_knowledge`; rejection/disable and
  restart persistence match existing Source lifecycle behavior.
- Same-name different-root Vaults remain distinct.
- Duplicate/overlapping roots are detected; two overlapping roots cannot both
  be confirmed without resolving ownership.
- Unknown `source_kind` values fail safely and never coerce to Agent Memory.
- Existing Agent Source IDs/fingerprints remain byte-for-byte stable after the
  additive migration.

##### Story 6.2 — Index Obsidian Markdown in an independent Knowledge schema

**As Carver,** I want confirmed Vault Markdown indexed without modifying it,
**so that** I can query my knowledge while the Vault remains authoritative.

**Acceptance criteria**

- Recursively enumerate regular `.md` notes under non-hidden paths.
- Exclude `.obsidian/**`, every dot-path, `.git/**`, trash, `.canvas`,
  attachments, binaries, plugin data, symlink directories, and root-escaping
  file symlinks without diagnostic noise for expected exclusions.
- Persist file-level Knowledge records in `knowledge_records` with `krec_`
  identity, Vault-relative locator, parser version, source revision, title/body,
  modified time, and complete provenance.
- Do not reuse `ProviderMemoryType`, `native_project`, or `memory_records`.
- Define a safe per-note byte bound from the measured corpus before
  implementation; an oversized note is reported without unbounded allocation.
- Preserve atomic generation/fencing and `dirty_after_validation` rules.
- Controlled success, failure, cancellation, retry, and rebuild fixtures show
  no path/byte/size/mtime change anywhere in the Vault.
- Additive migration preserves all Agent records, mappings, Sources, and
  queries.

##### Story 6.3 — Knowledge Inventory, health, and bounded Vault reconcile

**As Carver,** I want to see each Vault's scope and health and have changes
refresh safely, **so that** I know whether the Knowledge Index is trustworthy.

**Acceptance criteria**

- Knowledge Inventory shows Vault, path, source kind, coverage, health, complete
  Markdown-note count, last success, stale state, and safe error.
- One Vault failure never changes another Vault or Agent Source health.
- Only in-matrix Markdown events create dirty hints; `.obsidian` workspace
  changes and attachment/plugin activity create no scan.
- Burst saves are coalesced; disabled Sources do not reconcile.
- Periodic self-heal uses a measured bounded cadence and a lightweight no-op
  path; it does not inherit the existing 60-second full-rescan assumption.
- Move/permission/identity changes preserve the old Source as degraded and
  produce a Candidate requiring explicit rebind.
- Failed scans preserve the last-success generation and mark it stale.

##### Story 6.4 — Browse, keyword-search, and filter across Vaults

**As Carver,** I want to browse and search all confirmed Vaults and filter the
scope, **so that** I can find knowledge without switching Vaults manually.

**Acceptance criteria**

- Knowledge Browse supports all confirmed Vaults, one Vault, and
  Vault→folder→note drill-down.
- Knowledge Search defaults to all confirmed Obsidian Vaults and never
  implicitly adds Agent Memory.
- Filters support Vault/Source, folder prefix, and absolute modified-time
  threshold; the effective range is visible.
- Search and Browse share Knowledge provenance, cursor, stable ordering,
  EmptyState, coverage, and health semantics.
- Cursors bind active generations, Vault/folder/time filters, and policy
  revision; changes return stale cursor and restart at page one.
- Same-name notes in different Vaults remain separate.
- Chinese two/three-character literal queries and operator-like text are tested.
- Tag/property/backlink/semantic filtering is absent rather than simulated.

##### Story 6.5 — Show Knowledge Provenance and open the note in Obsidian

**As Carver,** I want every result to show its exact Vault/path and open in
Obsidian, **so that** I can verify and continue with the original note.

**Acceptance criteria**

- Result card displays Knowledge domain, Vault, Source, relative path, derived
  title/snippet, modified time, observed time, coverage, and health.
- Browser sends only `krec_` ID.
- Core resolves the active record, revalidates root containment, and constructs
  only an encoded `obsidian://open` URI using Vault ID + relative note path.
- Unicode, spaces, and reserved characters `# % ? & + /` are encoded correctly.
- No write-capable action or parameter can be represented.
- Same-name Vaults, missing target, moved Vault, unregistered URI handler, and
  dispatch failure return safe errors and never fabricate success.
- Automated tests prove URI construction/dispatch; human E2E verifies the
  correct note is visible in real Obsidian.
- The acceptance report states that Obsidian may update its own
  `.obsidian/workspace.json`; note/attachment mutation remains forbidden and
  that event triggers no Tessera scan.

##### Story 6.6 — Real multi-Vault acceptance and performance gate

**As Carver,** I want the feature validated against my real multi-Vault corpus,
**so that** it is trustworthy at the scale I actually use.

**Acceptance criteria**

- Test 0/1/6 Vault discovery plus corrupt registry, duplicate root, same-name
  Vault, nested root, move, permission loss, and explicit rebind.
- Test path traversal, non-UTF-8 names, symlink escape/cycle/retarget,
  mid-scan mutation, oversized note, and continuous-save drift.
- Test one failing Vault while all other Vaults and Agent Memory remain usable.
- Verify cross-Vault browse/search/filter, provenance, correct Obsidian open,
  offline use, keyboard paths, stale results, and Knowledge-only rebuild.
- Record cold scan, no-op reconcile, single-note freshness, keyword query
  P50/P95, RSS, index size, file descriptors, and thread count on the current
  six-Vault/1,813-note corpus.
- Lock thresholds only after measurement. If literal `instr` search fails the
  gate, introduce and remeasure a Knowledge-specific FTS implementation.
- Produce explicit evidence separating automated URI dispatch from real visible
  Obsidian open.

### 5.6 UX changes embedded in Epic 6

Add:

- **UX-DR9 — Domain separation:** top-level `Agent Memory` and
  `Obsidian Knowledge` destinations; no ambiguous mixed list.
- **UX-DR10 — Vault onboarding:** discovered Vault cards show name/path/basis
  and independent confirm/reject state; discovery errors are visible.
- **UX-DR11 — Knowledge Inventory:** attention-first health summary and one card
  per Vault with count, last scan, stale/error, browse, and rescan actions.
- **UX-DR12 — Knowledge navigation:** breadcrumb
  `Knowledge Sources → Obsidian → Vault → Folder → Note`.
- **UX-DR13 — Knowledge search/filter:** all confirmed Vaults by default;
  visible Vault/folder/time scope; no Agent-specific "Memory type" label.
- **UX-DR14 — Knowledge result/open:** Knowledge Provenance card and
  "Open in Obsidian" action with opening/error status.

All new flows retain the existing semantic focus order, keyboard activation,
status announcement, EmptyState, and stale-cursor recovery contracts.

### 5.7 Sprint-status change after approval

**OLD**

> Epic 1..5 are `done`; no Epic 6 entries.

**NEW**

```yaml
  # === Epic 6: 只读 Obsidian 多 Vault 知识联邦 ===
  epic-6: backlog
  6-1-obsidian-vault-discovery-confirmation: backlog
  6-2-obsidian-readonly-knowledge-index: backlog
  6-3-obsidian-inventory-health-reconcile: backlog
  6-4-cross-vault-browse-search-filters: backlog
  6-5-obsidian-provenance-open-original: backlog
  6-6-obsidian-multi-vault-acceptance-gate: backlog
  epic-6-retrospective: optional
```

Existing status lines remain unchanged.

## 6. Implementation Handoff

### 6.1 Handoff classification

**Major change:** route to Product Manager / Solution Architect first, then
Product Owner / Developer after the artifact edits and architecture gate are
approved.

### 6.2 Responsibilities

| Recipient | Responsibility |
| --- | --- |
| Product Manager / Product Owner | Approve Phase C.0 scope, UJ-5, FR-19..FR-25, explicit non-goals, and default domain separation. |
| Solution Architect | Finalize AD-37..AD-40, Knowledge schema/identity, source-kind dispatch, API contract, rebuild isolation, event filtering, and open URI safety. |
| UX owner | Add UX-DR9..UX-DR14 and ensure Vault/folder language does not reuse Agent-only labels. |
| Developer | Implement Stories 6.1..6.6 on feature branches with PRs; preserve unrelated WIP and existing Agent behavior. |
| Test/verification owner | Execute the fixture, zero-mutation, security, real multi-Vault, real Obsidian-open, accessibility, and performance matrices. |

### 6.3 Implementation success criteria

The change is complete only when:

1. approved PRD, Addendum, Architecture, SPEC, Requirements Matrix, Epic, UX
   decisions, and sprint status agree;
2. every confirmed Vault is a distinct `local_knowledge` Source;
3. Knowledge records never enter `memory_records`;
4. only supported Markdown under confirmed roots is read;
5. Tessera performs zero Vault writes;
6. browse/search/filter/provenance work across the selected real Vaults;
7. the open action targets the correct visible note in real Obsidian;
8. `.obsidian` activity does not schedule a scan;
9. one Vault failure leaves all other Vaults and Agent Memory usable;
10. Agent Memory behavior and data remain regression-free;
11. measured performance gates pass without invented thresholds.

## 7. Approval and Routing Record

Carver approved the complete proposal with `C` on 2026-07-26.

The approved planning changes were applied to:

1. `prds/prd-tessera-2026-07-20/prd.md`;
2. `prds/prd-tessera-2026-07-20/addendum.md`;
3. `architecture/architecture-tessera-2026-07-20/ARCHITECTURE-SPINE.md`;
4. `../specs/spec-tessera/SPEC.md`;
5. `../specs/spec-tessera/requirements-matrix.md`;
6. `epics.md`;
7. `../implementation-artifacts/sprint-status.yaml`.

The change remains classified as **Major** and is routed to Product
Manager/Product Owner and Solution Architect ownership before Developer
implementation. Epic 6 and Stories 6.1..6.6 are now backlog; Story 6.1 is the
first Developer handoff after implementation-readiness validation.

No feature-code implementation, commit, push, or PR is included in this
Correct Course workflow.

## 8. Workflow Execution Log

| Date | Workflow | Event | Result |
| --- | --- | --- | --- |
| 2026-07-26 | `bmad-correct-course` | Trigger and batch mode confirmed | Multi-Vault Obsidian support constrained to inventory, browse, search, filter, provenance, and opening the original note; Tessera remains strictly read-only. |
| 2026-07-26 | `bmad-correct-course` | Complete proposal reviewed | Approved by Carver with `C`. |
| 2026-07-26 | `bmad-correct-course` | Planning artifacts updated | PRD, Addendum, Architecture Spine, SPEC, Requirements Matrix, Epics, and sprint backlog updated; Epics 1..5 preserved. |
| 2026-07-26 | `bmad-correct-course` | Handoff | Major change routed to PM/PO + Solution Architect; Developer entry point is Story 6.1 after readiness validation. |

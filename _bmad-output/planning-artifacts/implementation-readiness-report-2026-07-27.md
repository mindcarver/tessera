---
status: needs-work
date: 2026-07-27
completed: 2026-07-27
project: tessera
stepsCompleted:
  - step-01-document-discovery
  - step-02-prd-analysis
  - step-03-epic-coverage-validation
  - step-04-ux-alignment
  - step-05-epic-quality-review
  - step-06-final-assessment
includedDocuments:
  prd:
    - _bmad-output/planning-artifacts/prds/prd-tessera-2026-07-20/prd.md
    - _bmad-output/planning-artifacts/prds/prd-tessera-2026-07-20/addendum.md
  architecture:
    - _bmad-output/planning-artifacts/architecture/architecture-tessera-2026-07-20/ARCHITECTURE-SPINE.md
  epics:
    - _bmad-output/planning-artifacts/epics.md
  ux: []
---

# Implementation Readiness Assessment Report

**Date:** 2026-07-27
**Project:** tessera

## Document Discovery

### PRD Documents

Canonical inputs:

- `prds/prd-tessera-2026-07-20/prd.md` — 38,560 bytes, modified
  2026-07-26 23:28:54
- `prds/prd-tessera-2026-07-20/addendum.md` — 9,687 bytes, modified
  2026-07-26 23:28:22

The reconciliation and rubric files in the same bundle are historical support
material and are excluded from the canonical assessment.

### Architecture Documents

Canonical input:

- `architecture/architecture-tessera-2026-07-20/ARCHITECTURE-SPINE.md` —
  37,384 bytes, modified 2026-07-26 23:29:43

The reconciliation and review files in the same bundle are historical support
material and are excluded from the canonical assessment.

### Epics and Stories Documents

Canonical input:

- `epics.md` — 61,457 bytes, modified 2026-07-26 23:27:12

### UX Design Documents

No standalone UX document was discovered by the configured filename patterns.
UX alignment will therefore be assessed against any UX requirements embedded
in the selected canonical artifacts.

### Discovery Issues

- No whole-versus-sharded duplicate was found.
- No traditional sharded `index.md` was found for these document types.
- The missing standalone UX artifact is a warning, not a duplicate-resolution
  blocker.

## PRD Analysis

### Functional Requirements

#### Agent Memory domain

- **FR-1 — 自动发现 Candidate Source：** 用户启动 Tessera 后，系统可以自动发现当前本机受支持的 Codex 与 Claude Code Candidate Source。发现结果标明 Provider、候选路径、发现依据和可判定的 Native Project 信息；发现阶段不读取原始聊天记录；自动发现没有结果时，MVP 不显示手动添加目录入口。
- **FR-2 — 确认或拒绝 Source：** 用户可以逐个确认或拒绝 Candidate Source；只有 Confirmed Source 才能进入正文扫描和索引流程。未确认或被拒绝的 Candidate Source 不进入 Derived Index；用户可以停用已确认的 Source，且停用不修改或删除原始 Agent Memory；确认记录能在本机应用重启后保留。
- **FR-3 — 查看 Source Inventory：** 用户可以在 Source Inventory 查看每个 Source 的 Provider、路径、Native Project、Coverage Level、Source Health、最近成功扫描时间、记录数量和最近错误。数量只在 Connector 能完整枚举时展示为完整数量；`search_only`、`existence_only` 或 `unsupported` 不被展示成“完整同步”；Source Health 变化不会删除用户已确认关系。
- **FR-4 — 保留 Native Project：** 系统可以按 Provider 原样保留每条 Agent Memory 的 Native Project，不把无法验证的目录键自动解释成真实 repository。无法确认的项目映射明确显示为未映射，而不是被猜测归类；同一 Native Project 下的 Agent Memory 可独立搜索。
- **FR-5 — 建立 Tessera Project 映射：** 用户可以创建 Tessera Project，并将 Codex 与 Claude Code 的一个或多个 Native Project 关联到同一 Tessera Project。映射仅存在于 Tessera 本地状态，不修改 Provider 目录或文件；用户可以查看、调整或移除映射；移除映射不会删除任何 Agent Memory 或 Derived Index 记录。
- **FR-6 — 限定 Agent Memory 边界：** Connector 只能把 Provider 自动生成的 Agent Memory 纳入 Derived Index。原始聊天、session transcript、完整对话消息不进入 Derived Index；`CLAUDE.md`、`AGENTS.md`、项目规则和其他人工指令文件不进入 MVP Derived Index；每条记录标明 Provider 内的 Agent Memory 类型，不能仅凭正文内容猜测类型。
- **FR-7 — 以只读方式建立 Derived Index：** 系统可以从 Confirmed Source 建立 Derived Index，并在任何扫描和重建过程中保持原始 Agent Memory 不变。扫描前后源文件集合、内容、大小和修改时间保持不变；删除 Tessera Derived Index 后可从 Confirmed Source 重新建立；失败扫描不会用不完整结果替换上一成功版本。
- **FR-8 — 更新 Derived Index：** 系统可以检测 Confirmed Source 的变化并更新 Derived Index，用户也可以手动触发 Source 重新扫描。新增、修改和删除的 Agent Memory 在成功扫描后反映到查询结果；扫描过程和最终状态对用户可见；手动重新扫描只作用于用户指定的 Confirmed Source。
- **FR-9 — 搜索 Confirmed Source：** 用户可以输入关键词，在全部或指定 Confirmed Source 的 Derived Index 中查询 Agent Memory。默认搜索所有健康且已成功索引的 Confirmed Source；查询不调用外部模型或远程搜索服务；空结果区分“确实无匹配”“Source 未索引”“Source 当前不可用”。
- **FR-10 — 筛选搜索结果：** 用户可以按 Provider、Confirmed Source、Tessera Project、Native Project、Agent Memory 类型和时间筛选结果。组合筛选条件时，界面显示当前生效范围；清除筛选后恢复全部 Confirmed Source 范围。
- **FR-11 — 展示原始结果与 Provenance：** 每条搜索结果必须展示原始 Agent Memory 片段及完整 Provenance，而不是自动生成的总结。每条结果至少包含 Provider、Source、Native Project、原始文件或 Provider 引用、定位信息和来源更新时间；结果明确显示 Coverage Level 与 Source Health；Tessera 不把推断标题或项目映射伪装成 Provider 原始事实。
- **FR-12 — 打开原始位置：** 用户可以从结果卡片打开或定位 Provenance 指向的原始 Agent Memory。Tessera 只打开或定位，不在应用内编辑原始文件；打开或定位由本地服务在校验路径边界后调用 OS 能力完成，浏览器本身不直接访问文件系统；原始位置失效时展示可理解的错误和 Source Health 状态。
- **FR-13 — 展示 Source Health：** 系统可以把每个 Confirmed Source 标记为 `unknown`、`healthy`、`degraded` 或 `error`，并给出可理解的原因。状态至少区分路径失效、权限不足、格式不支持和扫描失败；错误展示不包含 Agent Memory 正文或凭据。
- **FR-14 — 隔离 Connector 失败：** 一个 Connector 或 Confirmed Source 失败时，用户仍可搜索其他可用 Source。单个失败不会导致全局搜索不可用；失败 Source 的上一成功结果若继续展示，必须标明上次成功时间和 stale 状态。
- **FR-15 — 重建 Derived Index：** 用户可以删除并完整重建 Tessera Derived Index，而不影响 Confirmed Source 和 Tessera Project 映射。重建前明确告知只会删除 Tessera 派生数据；重建后可恢复相同来源记录的稳定身份和 Provenance；重建失败时原始 Agent Memory 保持不变。
- **FR-16 — 浏览 Agent Memory 集合：** 用户可以从 Source Inventory 或 Tessera Project 进入记忆集合，查看分页列表、最近变化和按条件筛选的 Agent Memory。浏览结果与搜索结果使用同一 Provenance、Coverage Level 和 Source Health 字段；空集合明确区分“尚未扫描”“没有可索引 Agent Memory”和“Source 当前不可用”；浏览列表不包含原始聊天、人工指令文件或未经确认的 Source。
- **FR-17 — 可视化记忆结构：** 用户可以通过列表、分组和状态视图理解各 Provider、Tessera Project、Native Project 和 Agent Memory 类型之间的关系。用户能从 Provider 进入项目，再进入记忆条目和原始位置；视图显示最近扫描、最近变化和 Source Health，而不把派生索引状态伪装成源数据状态；首版不要求知识图谱、关系自动推断或 AI 生成摘要。
- **FR-18 — 本地启动与使用：** 用户可以在本机启动 Tessera，完成发现、确认、扫描、搜索、打开来源和重建索引的完整闭环。MVP 正常使用不要求注册、登录或配置 Tessera 云服务；断网状态下，文件型 Codex 与 Claude Code Source 的全部 MVP 功能仍可使用；应用退出并重启后，Confirmed Source、Tessera Project 和 Derived Index 仍然可用。

#### Phase C.0 Obsidian Knowledge domain

- **FR-19 — Discover and confirm multiple Obsidian Vaults:** Tessera can discover local Obsidian Vault Candidates and allows the user to confirm each Vault independently before any note content is read. Discovery reads registered Vault metadata without reading note bodies; each existing registered Vault produces one Candidate with provider, native Vault identity, path, discovery basis, and Coverage Level; a missing, corrupt, or unsupported registry produces a visible diagnostic and is not presented as “Obsidian has no Vaults”; a native directory-picker fallback is restricted to selecting an existing Obsidian Vault and Tessera provides neither free-form arbitrary path input nor recursive HOME discovery; only a Confirmed Vault may be scanned or indexed.
- **FR-20 — View Knowledge Source Inventory:** The user can inspect every Candidate and Confirmed Obsidian Vault as an independent Knowledge Source. Inventory displays source kind, provider, Vault name, confirmed path, Coverage Level, Source Health, last successful scan, stale state, and latest safe error; when coverage is `full`, inventory displays the complete supported Markdown-note count; Vaults with the same display name remain distinct through native Vault identity and confirmed root; `.obsidian/**` and every other excluded artifact are absent from the note count.
- **FR-21 — Build a read-only Knowledge Index:** Tessera can recursively index supported regular Markdown notes inside a Confirmed Vault without modifying the Vault. Knowledge records use a separate record identity, table, parser, and migration namespace from Agent Memory; initial Knowledge Record granularity is one Markdown file per record; a note retains stable identity while its Confirmed Source and normalized Vault-relative Path remain unchanged, while renaming or moving a note creates a new locator; a successful, failed, cancelled, or retried scan and a Knowledge rebuild do not change any Vault file, byte, size, or mtime; a failed or drifting scan never replaces the previous successful generation.
- **FR-22 — Browse, search, and filter across confirmed Vaults:** The user can browse or keyword-search all Confirmed Obsidian Vaults or a selected Vault. Knowledge queries support filters for Vault, Vault-relative folder prefix, and source modification time; the interface displays the effective query scope; empty states distinguish no match, not indexed, and Source unavailable; Agent Memory is not implicitly included in the Knowledge result set.
- **FR-23 — Display Knowledge Provenance:** Every Knowledge result exposes enough Provenance to identify and verify the original note. Each result includes source kind, provider, Source ID, native Vault ID and name, Vault-relative Path, source modification time, observed time, Coverage Level, and Source Health; titles and snippets are explicitly derived presentation and are never represented as Obsidian-authored metadata unless that origin is verified; Tessera does not automatically summarize, merge, deduplicate, or resolve conflicts between notes.
- **FR-24 — Open the original note in Obsidian:** The user can ask Tessera to open a Knowledge result in Obsidian without granting the browser an arbitrary URI or filesystem capability. The browser submits only a trusted Knowledge `record_id`; the Rust core resolves the active record, verifies that the current target remains inside the Confirmed Vault, and constructs an encoded `obsidian://open` URI from the native Vault ID and Vault-relative Path; Tessera cannot construct any write-capable URI action or parameter, including `new`, `append`, `prepend`, `overwrite`, or content-bearing parameters; a missing note, moved Vault, unavailable URI handler, or dispatch failure produces a safe error and is never reported as successful opening.
- **FR-25 — Reconcile, isolate failures, and rebuild the Knowledge Index:** Tessera can keep Knowledge results current while isolating each Vault and the Agent Memory domain. Watcher events are hints and only supported Markdown changes may schedule a Vault reconcile; `.obsidian/**`, `.git/**`, trash, attachments, Canvas, plugin data, and all other excluded paths do not schedule a scan; failure of one Vault never blocks other Vaults or Agent Memory; a failed Vault keeps its previous successful generation available and clearly marked as stale; rebuilding Knowledge Records does not delete Agent Memory records, Confirmed Sources, or Tessera Project mappings.

**Total Functional Requirements: 25**

### Non-Functional Requirements

- **NFR-1 — Data ownership:** Agent Memory and supported Knowledge files always remain the facts in their respective Confirmed Sources; every Tessera Derived Index is only a rebuildable view.
- **NFR-2 — Privacy and no upload:** Normal local operation must not upload Agent Memory, Knowledge Source content, search queries, project mappings, Vault metadata, or diagnostics to Tessera or any third-party server.
- **NFR-3 — Redacted logs:** Application logs must not record Agent Memory bodies, note bodies, search queries, credentials, or unredacted source paths by default.
- **NFR-4 — Remote authorization:** Future remote Knowledge Sources may be connected by the local application only after explicit user configuration and authorization; they must not silently change the MVP privacy promise.
- **NFR-5 — Minimum read scope:** Tessera may read only user-confirmed Source boundaries and must never expose arbitrary filesystem or URI construction capabilities to the interface.
- **NFR-6 — Continuous path-boundary validation:** Any Agent Memory or Knowledge Source path change, symbolic link, root overlap, or permission change must pass path-boundary validation again.
- **NFR-7 — Untrusted content safety:** All displayed Agent Memory and Knowledge content is untrusted text; Tessera must not execute embedded HTML, scripts, commands, or URI actions.
- **NFR-8 — Failure isolation:** Failure of one Source must not block search or browse for any other Agent Memory or Knowledge Source.
- **NFR-9 — Atomic visible generation:** Every Source scan must use complete success as the visible-generation switch condition; failure preserves the previous successful Derived Index and marks it stale when shown.
- **NFR-10 — Recoverability:** If Tessera-owned indexes are corrupt or deleted, Agent and Knowledge indexes can be rebuilt independently from their Confirmed Sources without source mutation.
- **NFR-11 — Measured performance:** Query latency, cold scan time, no-op reconcile time, incremental update time, memory, file descriptors, threads, and index size must be measured with Carver's real Agent Memory and Obsidian datasets before fixed thresholds are adopted.
- **NFR-12 — Non-blocking scan:** Scanning any Source must not block queries against its previous successful Derived Index.
- **NFR-13 — Keyboard accessibility:** Core discovery, inventory, browse, search, filter, Provenance, and source-opening actions in both domains must be keyboard accessible.
- **NFR-14 — Vault zero-write:** Tessera's discovery, confirmation, scan, search, browse, filter, health, reconcile, and rebuild paths never write inside an Obsidian Vault. A user-triggered open delegates to Obsidian; Obsidian-owned `.obsidian` workspace changes are excluded from the Knowledge Index and must not schedule a Tessera reconcile.

**Total Non-Functional Requirements: 14**

### Additional Requirements

#### Product scope and domain boundaries

- Phase A remains the local Codex and Claude Code Agent Memory MVP; Phase C.0 is an additive local Obsidian multi-Vault Knowledge expansion; Phase C.1+ defers RAGFlow, Feishu, and explicit mixed-domain queries.
- Agent Memory and Knowledge Sources remain separate domains. Phase C.0 uses one independent `local_knowledge` Source per confirmed Vault and does not mix Agent Memory into Knowledge queries by default.
- Phase C.0 supports regular Markdown notes only, one Knowledge Record per file, Knowledge Inventory, browse, keyword search, Vault/folder/source-modification-time filters, Knowledge Provenance, safe open in Obsidian, source-scoped reconcile, stale last-success behavior, and Knowledge-only rebuild.
- Phase C.0 excludes note/Vault creation, editing, rename, move, delete, append, prepend, overwrite, frontmatter/property/tag/link mutation, `.obsidian` mutation, Sync/cloud/team features, attachments/OCR/PDF, Canvas/Bases/plugin databases/trash, structured tag/property/backlink/embed/graph semantics, AI/embedding/semantic retrieval, summaries/deduplication/conflict resolution, RAGFlow, Feishu, arbitrary directories, HOME-wide discovery, and default mixed Agent Memory + Knowledge search.

#### Deployment, permission, and data constraints

- The delivered application is a local Web application: Rust core embeds a loopback-only HTTP server bound to `127.0.0.1`; the browser hosts the React UI.
- Every endpoint uses a versioned `api_version` contract and accepts trusted `source_id` or `record_id` identifiers rather than arbitrary filesystem paths, URIs, SQL, shell capabilities, or provider credentials.
- The loopback service validates Host and Origin and returns a restrictive CSP response header; normal operation has no remote endpoint or telemetry.
- Rust core is the only filesystem, Source Registry, connector, scan, index, mapping, query, and open boundary. Browser code has no direct filesystem capability.
- SQLite FTS5 is a deletable/rebuildable derived index. Any external SQLite source must be demonstrably opened without write sidecars or the connector must use an official file representation or degrade safely.
- Scan generation staging, fencing, final manifest validation, and atomic active-generation switching prevent partial results. Content hash detects change rather than serving as the sole record identity.
- Watcher events only produce dirty hints. Reconcile and self-healing own truth; excluded `.obsidian` or plugin/attachment activity cannot schedule Knowledge work.
- Markdown and all indexed content are rendered as untrusted data.

#### Supported artifact constraints

- Codex includes supported automatically generated Markdown such as `MEMORY.md`, `memory_summary.md`, `raw_memories.md`, and `rollout_summaries/*.md`; it excludes raw rollout/transcript JSONL, session content, and conversation state databases.
- Claude Code includes `MEMORY.md` and topic Markdown under project auto-memory directories; it excludes `CLAUDE.md`, `AGENTS.md`, `.claude/rules`, session, and transcript content.
- Obsidian discovery includes registered Vault metadata or a constrained native picker for an existing Vault; it excludes note-body reads during discovery, arbitrary paths, HOME scans, and remote Vault APIs.
- Obsidian content includes regular Markdown under non-hidden Vault-relative paths; it excludes `.obsidian/**`, all dot paths, `.git/**`, trash, `.canvas`, attachments, binaries, plugin data, symlink directories, and root-escaping file symlinks.
- The local Obsidian registry is an observed integration surface rather than a documented stable API. Its parser requires fixtures and visible failure diagnostics.
- Confirmed nested or overlapping Vault roots are blocked until the user resolves ownership.

#### Open and validation constraints

- Open-in-Obsidian accepts only a trusted Knowledge record ID. Rust core resolves the active record, revalidates containment, and constructs only `obsidian://open?vault=<id>&file=<relative-path>` with percent-encoded dynamic fields.
- The open constructor cannot represent `new`, `append`, `prepend`, `overwrite`, `content`, or any write-capable action/parameter. Dispatch success is distinct from human-visible proof that Obsidian opened the intended note.
- Opening may cause Obsidian itself to update `.obsidian` workspace state; Tessera neither performs that write nor indexes or reconciles from the event.
- Before FR-21 implementation starts, the team must define a bounded maximum Markdown note-size policy using real Vault distribution and security requirements. Oversized notes receive a safe diagnostic without replacing the previous generation.
- Performance thresholds must be selected only after measuring cold scan, no-op reconcile, incremental update, query latency, RSS, file descriptors, threads, and index size on the verified real corpus. The PRD records 1,813 Markdown notes as the current Phase C.0 baseline.
- Real acceptance must distinguish automated URI dispatch from human-visible correct-note opening and Tessera-originated writes from concurrent Obsidian or sync-tool activity.

#### Assumptions and unresolved decisions

- Phase A's first validation environment is Carver's current macOS machine; cross-platform delivery is not a Phase A acceptance condition.
- Keyword retrieval is assumed sufficient to prove first-release value; semantic retrieval requires later evidence.
- The note-size ceiling and all Phase C.0 performance thresholds remain explicit preimplementation decisions rather than invented values.
- The Obsidian registry format, Vault-ID behavior after move/rename, and URI-handler availability require fixture or real-environment validation.
- A standalone UX artifact does not exist; approved UX decisions are embedded in `epics.md`.

### PRD Completeness Assessment

The PRD provides a numbered, testable requirement set of 25 FRs and 14 NFRs,
explicit phase boundaries, supported/excluded artifact matrices, user journeys,
success metrics, risks, non-goals, and a local-only trust model. Phase C.0 is
defined as an additive read-only Knowledge domain rather than a general-purpose
filesystem or a third Agent provider.

The PRD is sufficiently explicit for Epic coverage validation, with two
intentional preimplementation gates still open: the bounded maximum Markdown
note size and measured performance thresholds for the real Vault corpus.
Visible correct-note opening in real Obsidian is also a human E2E evidence
requirement rather than something an automated URI-dispatch test can prove.

## Epic Coverage Validation

### Epic FR Coverage Extracted

- Epic 1 covers FR-1, FR-2, FR-3, FR-4, FR-6, FR-7, FR-8, FR-9, FR-11,
  FR-12, FR-13, and FR-18.
- Epic 2 completes multi-Provider coverage for FR-3, FR-9, FR-10, FR-13, and
  contributes to FR-14.
- Epic 3 covers FR-16 and FR-17.
- Epic 4 covers FR-8, FR-13, FR-14, and FR-15.
- Epic 5 covers FR-5 and completes the Tessera Project projection used by
  FR-10.
- Epic 6 covers FR-19 through FR-25.

**Total distinct FRs represented in the Epic coverage map: 25**

### Coverage Matrix

| FR | PRD requirement | Epic and Story coverage | Status |
| --- | --- | --- | --- |
| FR-1 | Automatically discover supported Codex and Claude Code Candidates without reading chat | Epic 1, Stories 1.2 and 2.1 | Covered |
| FR-2 | Confirm, reject, disable, and persist Source decisions | Epic 1, Story 1.3; Epic 2, Story 2.1 reuses the lifecycle | Covered |
| FR-3 | Show truthful Source Inventory, Coverage, Health, count, scan time, and error | Epic 1, Story 1.8; Epic 2, Story 2.5 | Covered |
| FR-4 | Preserve Provider-native project identity without guessing | Epic 1, Story 1.5; Epic 5, Story 5.1 preserves mapping boundaries | Covered |
| FR-5 | Create Tessera Projects and explicitly map Native Projects | Epic 5, Stories 5.1 and 5.2 | Covered |
| FR-6 | Admit only supported Agent Memory, excluding chats and human instructions | Epic 1, Story 1.5; Epic 2, Story 2.2 | Covered |
| FR-7 | Build a zero-source-mutation, rebuildable Derived Index with atomic visibility | Epic 1, Stories 1.4 and 1.5 | Covered |
| FR-8 | Refresh the Derived Index automatically and by Source-scoped manual rescan | Epic 1, Story 1.8; Epic 4, Story 4.1 | Covered |
| FR-9 | Search all or selected healthy Confirmed Agent Sources with truthful empty states | Epic 1, Story 1.6; Epic 2, Story 2.3 | Covered |
| FR-10 | Filter Agent results by Provider, Source, project, type, and time with visible scope | Epic 2, Story 2.4; Epic 5, Story 5.2 | Covered |
| FR-11 | Show original Agent Memory snippets and complete Provenance | Epic 1, Story 1.6; Epic 2, Story 2.3 | Covered |
| FR-12 | Safely open the original Agent Memory location | Epic 1, Story 1.7 | Covered |
| FR-13 | Show structured Source Health and safe causes | Epic 1, Story 1.8; Epic 2, Story 2.5; Epic 4, Story 4.2 | Covered |
| FR-14 | Isolate Connector/Source failures and retain stale last-success | Epic 4, Story 4.2; Epic 2, Story 2.3 introduces the cross-source behavior | Covered |
| FR-15 | Reset and rebuild Derived Index without losing confirmations or project mappings | Epic 4, Story 4.4 | Covered |
| FR-16 | Browse Agent Memory collections without a search term | Epic 3, Stories 3.1 and 3.2 | Covered |
| FR-17 | Visualize and drill down the Agent Memory structure without invented semantics | Epic 3, Stories 3.2 and 3.3 | Covered |
| FR-18 | Complete the local, offline, restart-persistent product loop | Epic 1, Stories 1.1 through 1.9; shared by Epics 2 through 5 | Covered |
| FR-19 | Discover and independently confirm multiple Obsidian Vaults | Epic 6, Story 6.1 | Covered |
| FR-20 | Show truthful per-Vault Knowledge Inventory | Epic 6, Stories 6.1 and 6.3 | Covered |
| FR-21 | Build an independent, read-only Knowledge Index | Epic 6, Story 6.2 | Covered |
| FR-22 | Browse, search, and filter all or selected confirmed Vaults | Epic 6, Story 6.4 | Covered |
| FR-23 | Show complete Knowledge Provenance | Epic 6, Stories 6.4 and 6.5 | Covered |
| FR-24 | Open the trusted original note through fixed-action Obsidian URI dispatch | Epic 6, Story 6.5 | Covered |
| FR-25 | Reconcile Vault changes, isolate failure, retain stale data, and rebuild Knowledge independently | Epic 6, Stories 6.2, 6.3, and 6.6 | Covered |

### Missing Requirements

No PRD Functional Requirement is absent from the Epic coverage map or Story
set. No Epic FR identifier exists outside the PRD range FR-1 through FR-25.

### Coverage Statistics

- **Total PRD FRs:** 25
- **FRs covered in Epics and Stories:** 25
- **Missing FRs:** 0
- **Extraneous Epic FRs:** 0
- **Coverage:** 100%

## UX Alignment Assessment

### UX Document Status

**No standalone UX document exists.** Tessera is a user-facing local Web
application, so UX is required and cannot be treated as not applicable. The
user confirmed `epics.md` UX-DR1 through UX-DR14 as the embedded UX input for
this assessment.

### UX ↔ PRD ↔ Architecture Alignment

| UX decision | PRD alignment | Architecture support | Assessment |
| --- | --- | --- | --- |
| UX-DR1 Source discovery and confirmation | UJ-1, FR-1, FR-2 | AD-3, AD-4, AD-33, AD-35 | Aligned |
| UX-DR2 Source Inventory state cards and EmptyState | FR-3, FR-13 | AD-7, AD-13, AD-21, AD-23 | Aligned |
| UX-DR3 Tessera Project creation and mapping | FR-5 | AD-24, AD-27, AD-31 | Aligned |
| UX-DR4 Native scope preservation and isolation | FR-4 | AD-6, AD-24 | Aligned |
| UX-DR5 Agent search/filter scope and truthful empty states | FR-9, FR-10 | AD-7, AD-17, AD-21, AD-23, AD-26, AD-31 | Aligned |
| UX-DR6 Agent result card, Provenance, and open | FR-11, FR-12 | AD-4, AD-6, AD-17, AD-21 | Aligned |
| UX-DR7 Browse and structural drill-down | FR-16, FR-17 | AD-21, AD-23, AD-26 | Aligned |
| UX-DR8 Keyboard reachability and shared interaction contract | NFR-13 | AD-21 and `tests/ui/accessibility.spec.ts` | Aligned |
| UX-DR9 Separate Agent Memory and Obsidian Knowledge destinations | FR-22 and Phase C.0 non-goals | AD-10, AD-19, AD-39 | Aligned |
| UX-DR10 Vault onboarding and discovery diagnostics | UJ-5, FR-19 | AD-4, AD-33, AD-35, AD-37 | Aligned with implementation clarification required |
| UX-DR11 Knowledge Inventory and attention-first health | FR-20, FR-25 | AD-7, AD-13, AD-21, AD-39 | Aligned |
| UX-DR12 Knowledge breadcrumb and drill-down | UJ-5, FR-22 | AD-17, AD-21, AD-23, AD-26, AD-39 | Aligned |
| UX-DR13 Knowledge search/filter scope | FR-22 | AD-17, AD-19, AD-23, AD-26, AD-31, AD-39 | Aligned |
| UX-DR14 Knowledge Provenance and Open in Obsidian | FR-23, FR-24, NFR-14 | AD-4, AD-17, AD-21, AD-40 | Aligned |

### Alignment Issues

1. **Rust-owned Vault picker delivery is not explicit.** FR-19 and UX-DR10
   require a constrained native directory-picker fallback, while AD-1, AD-4,
   and NFR-5 forbid the browser from acquiring arbitrary filesystem
   capability. AD-37 defines the safety outcome but not the concrete
   interaction port. Story 6.1 implementation design must specify a Rust-owned
   OS-dialog adapter/endpoint that returns only a validated existing Vault
   Candidate; browser File System Access APIs and free-form path submission do
   not satisfy the contract.
2. **No fixed interaction-latency target exists.** NFR-11 and AD-22 correctly
   defer thresholds until real measurement. UX loading/progress behavior must
   therefore remain explicit for scan, search, and open operations until the
   measured gates are locked.

### Warnings

- The embedded UX decisions define behavior and accessibility contracts, but
  they do not provide wireframes, responsive layout rules, visual hierarchy,
  screen-level state diagrams, or usability-test evidence for Vault onboarding,
  Knowledge Inventory, multi-Vault filters, and open-error recovery.
- Accessibility coverage specifies keyboard paths, semantic focus order,
  readable status labels, and a Playwright artifact, but the absence of a
  standalone UX specification leaves detailed screen-reader announcements,
  contrast, zoom/reflow, and reduced-motion behavior to implementation review.
- These warnings do not contradict the PRD or Architecture, but they increase
  implementation and acceptance risk for the new user-facing Phase C.0 flows.

## Epic Quality Review

### Structural Summary

- **Epics reviewed:** 6
- **Stories reviewed:** 29
- **Story-format check:** all 29 Stories contain `As/I want/So that` intent,
  an Acceptance Criteria section, and Given/When/Then conditions.
- **Epic 6 dependency direction:** valid; it depends only on completed Epic 1,
  Epic 3, and Epic 4 capabilities and has no dependency on a future Epic.
- **Pre-Story 6.1 architecture gate:** satisfied by the selected final
  Architecture Spine updated 2026-07-26, which contains the approved
  Obsidian-specific AD-37 through AD-40. This readiness assessment is the
  remaining downstream planning gate.
- **Database/entity timing:** the `source_kind` compatibility work first
  appears with Vault onboarding in Story 6.1; `knowledge_records` first appears
  when it is used by Story 6.2. No up-front “create all future tables” Story is
  specified.
- **Project type:** Phase C.0 is a brownfield expansion of the implemented
  modular monolith, so a new starter-template Story is not required.

### Epic-Level Compliance

| Epic | User value | Independence | Story sequence | Assessment |
| --- | --- | --- | --- | --- |
| Epic 1 | Complete local Codex discovery/search/open loop | Standalone foundation | Generally sequential | Pass with historical terminology/sizing concerns |
| Epic 2 | Add Claude Code and cross-Provider use | Uses Epic 1 | Contains an explicit forward reference to Epic 5 | Historical structural violation |
| Epic 3 | Browse and understand memory without a query | Uses Epics 1–2 | No forward dependency | Pass |
| Epic 4 | Diagnose failure and recover indexes | Uses Epics 1–2 | No forward dependency | Pass |
| Epic 5 | Create cross-Agent Tessera Project views | Uses Epics 1–2 | Story 5.2 follows 5.1 | Pass |
| Epic 6 | Manage multiple Obsidian Vaults read-only | Uses completed platform capabilities | No future-Epic dependency, but several Stories are oversized or have unresolved acceptance gates | Needs refinement before development |

### Critical Violations

No unresolved future-Epic dependency blocks Epic 6. One historical planning
violation remains in a completed Epic:

1. **Story 2.4 explicitly defers part of its promised filter behavior to Epic
   5.** `epics.md:420` says “Tessera Project 筛选位预留（Epic 5 填充）”.
   Forward dependencies violate Epic independence. Epic 5 is now complete, so
   this is not an implementation blocker for Epic 6, but the canonical planning
   text should attribute the Tessera Project filter directly to Story 5.2
   rather than presenting Epic 2 as incomplete.

### Major Issues

1. **Story 6.1 combines too many independently risky changes.** It includes
   additive persisted `source_kind` compatibility, fail-closed unknown-kind
   handling, OS-specific registry parsing, constrained fallback picker
   delivery, Source confirmation lifecycle, same-name identity, overlap
   rejection, and Agent-ID regression protection. This is larger than a
   normal independently reviewable Story. Before `create-story`, either split
   compatibility/migration from Vault onboarding or define explicit,
   separately reviewable implementation checkpoints with an end-to-end
   Candidate/confirm slice as the only completion boundary.
2. **Story 6.2 is an Epic-sized indexing slice.** It owns independent schema and
   migration, identity, enumerator exclusions, parser, bounded reads, atomic
   generation/fencing, zero-mutation proofs, and Agent-regression migration
   evidence. Split schema/migration/identity from the read-only
   enumerate/parse/index pipeline, or the Story will be difficult to review,
   recover, and verify atomically. The approved change proposal requires the
   maximum note-size decision before Story 6.2 implementation, while the
   current AC derives it inside Story 6.2; a named pre-Story decision artifact
   is therefore still required.
3. **Story 6.3 combines two user outcomes and contains a measurement-dependent
   acceptance condition.** Knowledge Inventory UI and bounded
   watcher/reconcile/rebind/recovery are separable value slices. Its
   “measured bounded cadence” criterion at `epics.md:664` has no value or
   evidence artifact until the later performance gate. Move threshold locking
   to the measurement Story and make the functional reconcile Story require a
   bounded/configurable cadence plus a deterministic no-op path.
4. **Story 6.6 embeds conditional implementation inside an acceptance gate.**
   At `epics.md:762`, failure of literal `instr` search causes the same Story to
   introduce Knowledge-specific FTS and remeasure. That creates unbounded,
   result-dependent scope. The gate should fail truthfully and create an
   explicit remediation Story for FTS, which is then independently designed,
   implemented, and rerun against the same evidence set.

### Minor Concerns

1. **Vault-picker interaction port is underspecified.** Story 6.1 requires a
   constrained picker at `epics.md:589` but does not state that the OS dialog
   is Rust-owned and that the browser submits neither a directory path nor a
   browser-granted directory handle.
2. **The per-note byte policy lacks a named evidence artifact.** Story 6.2 at
   `epics.md:629` requires a safe bound but does not say where the decision,
   corpus distribution, security rationale, and rejection fixtures are stored.
3. **The real-corpus performance artifact and privacy boundary are implicit.**
   Architecture names `tests/benchmarks/knowledge-index.json`, but Story 6.6
   does not require that stable path or explicitly prohibit committed note
   bodies, filenames, or Vault paths in the benchmark output.
4. **The human-visible Obsidian-open evidence contract is unnamed.** Story 6.5
   correctly distinguishes URI dispatch from visible success, but does not
   identify the manual evidence artifact, operator steps, or pass/fail record
   required for handoff.
5. **Story 1.1 retains obsolete desktop terminology.** The title and user
   statement at `epics.md:213-216` say “桌面应用骨架” although its AC and the
   adopted Architecture now specify a local Web application.
6. **Epic 2 dependency prose retains obsolete `IPC` terminology.** The current
   architecture uses versioned loopback HTTP and SSE; canonical Epic text
   should not imply the abandoned Tauri IPC transport.
7. **Story size is uneven.** Stories 6.4 and 6.5 are cohesive enough to remain
   vertical slices, but they are still substantially larger than most
   completed Stories and require task-level sequencing in their Story specs.

### Recommended Remediation Before Story 6.1 Development

1. Resolve the four Major issues through an Epic 6 refinement pass before
   marking any Story `ready-for-dev`.
2. Make the Rust-owned picker boundary and benchmark evidence paths explicit.
3. Keep Story 6.6 as a pure acceptance decision: pass, or fail with a named
   remediation Story; do not hide a search-engine migration inside the gate.
4. Preserve the approved user-facing order:
   safe Vault confirmation → independent read-only indexing → trustworthy
   health/reconcile and query → safe open → real acceptance.
5. Treat historical Epic 1/2 wording cleanup as documentation maintenance, not
   as a reason to reopen completed implementation.

## Summary and Recommendations

### Overall Readiness Status

## NEEDS WORK

The Phase C.0 product and architecture direction is coherent: all 25
Functional Requirements are covered, Agent Memory and Obsidian Knowledge remain
separate domains, the Vault zero-write boundary is explicit, and the approved
pre-Story 6.1 architecture gate is represented by final AD-37 through AD-40.

Implementation should not begin yet because four current Major Story-quality
issues make scope, review, and acceptance unpredictable. The issue is delivery
decomposition and evidence sequencing, not missing product intent or missing
architecture direction.

### Critical Issues Requiring Immediate Action

There is no current Critical requirement-coverage or architecture-boundary gap.
The following Major issues must nevertheless be resolved before any Epic 6
Story becomes `ready-for-dev`:

1. **Refine Story 6.1:** separate or explicitly checkpoint persisted
   `source_kind` compatibility/migration from registry discovery, Rust-owned
   picker delivery, confirmation lifecycle, overlap handling, and regression
   protection.
2. **Split Story 6.2 and resolve its precondition:** decide and document the
   maximum note-size policy before implementation, then separate
   schema/migration/identity from the read-only enumerate/parse/index pipeline
   and its zero-mutation evidence.
3. **Split and make Story 6.3 measurable:** separate Knowledge Inventory from
   watcher/reconcile/rebind/recovery; keep the functional Story deterministic
   and move cadence-threshold locking to named measurement evidence.
4. **Make Story 6.6 a pure gate:** a failed search-performance gate must create
   an explicit FTS remediation Story; it must not silently expand the
   acceptance Story into a search-engine migration.

### Recommended Next Steps

1. Run an Epic 6 refinement/correct-course pass that preserves FR-19 through
   FR-25 while producing smaller vertical Stories and updated sprint IDs.
2. Add a focused UX contract for Vault onboarding, Knowledge Inventory,
   multi-Vault filtering, loading/error states, keyboard and screen-reader
   behavior, overlap resolution, and Open-in-Obsidian feedback.
3. Make the Rust-owned native Vault-picker interaction contract explicit: the
   browser submits only an action request, and Rust returns a validated
   existing Vault Candidate rather than an arbitrary path or directory handle.
4. Create named evidence artifacts for the pre-Story note-size decision,
   Knowledge performance benchmark, and human-visible Obsidian-open E2E. Keep
   note bodies, private filenames, Vault paths, and registry payloads out of
   committed evidence.
5. Clean historical planning drift without reopening completed implementation:
   remove Story 2.4's forward dependency wording, replace obsolete `IPC`
   references, and rename Story 1.1's obsolete desktop-shell wording.
6. Rerun Implementation Readiness. Only after a `READY` result should the first
   refined Epic 6 Story be created and moved to `ready-for-dev`.

### Issue Summary

- **Current Major issues:** 4
- **Historical forward-dependency defect:** 1, already resolved in delivered
  functionality but still present in canonical planning text
- **Minor Epic/Story concerns:** 7
- **UX alignment issues:** 2
- **UX documentation warnings:** 3
- **Missing FR coverage:** 0
- **Unmet architecture gate:** 0

### Final Note

This assessment found four implementation-blocking issues across Story
decomposition and evidence sequencing, plus advisory UX and documentation
concerns. The Obsidian direction remains sound and strictly read-only. Refining
the Story boundaries now is materially cheaper than discovering the same
ambiguity during migration, watcher, or performance implementation.

**Assessment completed:** 2026-07-27
**Assessor:** Codex — Product requirements traceability and implementation
readiness review

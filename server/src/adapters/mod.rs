//! `adapters` — provider implementations of `domain::ports::ProviderAdapter`.
//!
//! Adapters are read-only against the Source root (AD-2/AD-4/AD-11) and must
//! emit the normalized canonical envelope (AD-25). Phase 0 only fixes the
//! module surface; the Codex adapter lands in Story 1.5 and the Claude Code
//! adapter in Epic 2.
//!
//! Fixture contract anchors (AD-14/AD-3) live at
//! `server/tests/fixtures/providers/{codex,claude_code}`.

pub mod claude_code;
pub mod codex;
pub mod opencode;
/// Shared, provider-agnostic Markdown canonicalizer + path/locator helpers
/// (Story 2.2 extraction). Re-exported by `codex` for backward compat; new
/// providers import from this module directly. One parser, many version tags.
pub mod markdown;
/// Read-only Obsidian Vault discovery (Story 6.2, Phase C.0). Parses the host
/// vault registry and emits `local_knowledge` Candidates. Knowledge uses an
/// independent pipeline (AD-19/AD-38); this module is NOT a `ProviderAdapter`.
pub mod obsidian;

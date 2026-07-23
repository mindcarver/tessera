//! `adapters` — provider implementations of `domain::ports::ProviderAdapter`.
//!
//! Adapters are read-only against the Source root (AD-2/AD-4/AD-11) and must
//! emit the normalized canonical envelope (AD-25). Phase 0 only fixes the
//! module surface; the Codex adapter lands in Story 1.5 and the Claude Code
//! adapter in Epic 2.
//!
//! Fixture contract anchors (AD-14/AD-3) live at
//! `server/tests/fixtures/providers/{codex,claude_code}`.

pub mod codex;
pub mod claude_code;

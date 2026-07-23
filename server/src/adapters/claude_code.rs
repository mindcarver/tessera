//! `adapters::claude_code` — Claude Code Provider adapter (Epic 2).
//!
//! Phase 0 reserves the module and the fixture seed. The adapter follows the
//! same ProviderAdapter contract and Supported Artifact Matrix (AD-11):
//! `~/.claude/projects/<project>/memory/` and the user-configured
//! `autoMemoryDirectory`'s `MEMORY.md` and topic Markdown only.
//! `CLAUDE.md`, `AGENTS.md`, `.claude/rules`, session/transcript content and
//! any manually-added directory are rejected before canonicalization.

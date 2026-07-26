/**
 * Tessera — provider display-name helper (Story 3.1).
 *
 * Extracted from both `src/features/search/Search.tsx` and
 * `src/features/sources/Sources.tsx` so every surface renders the same
 * display name for a given provider id. Codex / Claude Code are the only
 * known provider ids today; an unknown future id falls back to its raw wire
 * string (rather than hiding it) so a new provider is visible immediately,
 * even before its display name is mapped.
 *
 * @param provider  Stable lowercase provider id (`codex`, `claude_code`).
 * @returns Human-readable display name (`Codex`, `Claude Code`).
 */
export function providerDisplayName(provider: string): string {
  switch (provider) {
    case "codex":
      return "Codex";
    case "claude_code":
      return "Claude Code";
    case "opencode":
      return "OpenCode";
    default:
      return provider;
  }
}

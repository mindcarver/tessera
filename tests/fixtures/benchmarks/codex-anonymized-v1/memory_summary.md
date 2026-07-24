# Local Memory Design

This anonymized benchmark fixture describes a local-first memory explorer.
Source material is read-only, while the application rebuilds a Derived Index
for search. The system keeps provenance so a result can return to its original
local memory location without sending content to a remote service.

## Baseline Principles

- Confirm a source before scanning it.
- Keep the Derived Index replaceable and local.
- Make search and inventory facts observable without exposing source text.

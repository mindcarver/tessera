# Architecture Spine

The architecture uses a local-first core, a read-only source boundary, and a
rebuildable Derived Index. A scan validates its source boundary before an
active generation changes. Query results carry provenance and remain useful
when a later scan fails.

## Search Contract

Search is performed against the active local generation. The browser receives
safe result metadata while the core retains filesystem and index authority.

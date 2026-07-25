# Rust Notes

Second topic Markdown file for the Claude fixture. Anchors the multi-topic
contract test: the adapter must index every direct-child `*.md`, dedup by
relative path, and tag each non-`MEMORY.md` file as `topic_memory`.

## Ownership

Borrow, do not clone, when the lifetime is local.

## Error handling

Use `Result` for recoverable errors; `panic!` only for invariant violations.

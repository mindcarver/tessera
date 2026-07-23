//! FTS5 availability integration test (Phase 0 / spec-1-1).
//!
//! Asserts that the locked stack (`rusqlite` 0.40.1 with the `bundled`
//! feature) ships an SQLite 3.x build in which `CREATE VIRTUAL TABLE ...
//! USING fts5(...)` succeeds. FTS5 is a hard prerequisite for the Derived
//! Index search schema (AD-15) and the Phase 0 Deferred verification
//! recorded in `docs/phase-0-verification.md`. If this test fails on the
//! locked stack, the spec's Block If #2 has fired and the bootstrap must
//! stop — no fallback to a non-FTS5 build is acceptable.
//!
//! The test deliberately lives in `server/tests/` (integration tests)
//! rather than as a unit test inside the `index` module because FTS5
//! availability is a property of the bundled SQLite build, not of any
//! Rust module under test.

use rusqlite::Connection;

#[test]
fn fts5_virtual_table_can_be_created() {
    let conn = Connection::open_in_memory().expect("open in-memory db");

    // Exercise both the basic FTS5 vtable and a content-less FTS5 table,
    // since the Derived Index search schema (Stories 1.5/1.6) may use
    // content-less FTS5 to keep the canonical body out of the FTS layer.
    conn.execute_batch(
        r#"
        CREATE VIRTUAL TABLE fts5_basic USING fts5(
            title,
            body,
            tokenize = "unicode61"
        );

        CREATE VIRTUAL TABLE fts5_external USING fts5(
            title,
            body,
            content='',
            tokenize = "unicode61"
        );
        "#,
    )
    .expect("CREATE VIRTUAL TABLE ... USING fts5 must succeed on bundled SQLite");

    // Round-trip a row to make sure the tokenizer and the FTS5 query layer
    // both work, not just the DDL.
    conn.execute(
        "INSERT INTO fts5_basic(title, body) VALUES (?1, ?2)",
        ["hello", "world"],
    )
    .expect("insert into fts5_basic");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM fts5_basic WHERE fts5_basic MATCH ?1",
            ["hello OR world"],
            |row| row.get(0),
        )
        .expect("fts5 MATCH query");
    assert_eq!(count, 1, "FTS5 MATCH must find the inserted row");
}

#[test]
fn fts5_trigram_tokenizer_is_available() {
    // The Phase 0 verification doc records the trigram-vs-unicode61 choice
    // for Chinese text. trigram is shipped with SQLite 3.34+; bundled
    // SQLite in rusqlite 0.40.1 is well past that. This test pins the
    // capability so a future stack bump that silently drops trigram trips
    // the spec's Block If here rather than during Story 1.6.
    let conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(
        r#"
        CREATE VIRTUAL TABLE fts5_trigram USING fts5(
            body,
            tokenize = "trigram"
        );
        "#,
    )
    .expect("trigram tokenizer must be available on bundled SQLite");

    // Round-trip a >=3-character CJK substring so this test pins that trigram
    // actually tokenizes CJK into queryable trigrams — not merely that the DDL
    // loads. The Phase 0 verification doc records that trigram needs >=3 chars
    // and does NOT match 1-2 char Chinese queries; that 1-2 char gap is a
    // Story 1.6 concern (see docs/phase-0-verification.md).
    conn.execute(
        "INSERT INTO fts5_trigram(body) VALUES (?1)",
        ["记忆管理与本地优先"],
    )
    .expect("insert CJK row");
    let hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM fts5_trigram WHERE fts5_trigram MATCH ?1",
            ["记忆管理"],
            |row| row.get(0),
        )
        .expect("trigram MATCH query");
    assert!(
        hits >= 1,
        "trigram must match a >=3-char CJK substring, got {hits} hits"
    );
}

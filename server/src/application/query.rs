//! Read-side orchestration for confirmed Sources and their current active index.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::domain::ports::query_store::QueryStore;
use crate::domain::query::{SearchEmptyState, SearchPage, SearchRequest, MAX_CURSOR_BYTES};
use crate::domain::scan::ScanRunState;
use crate::domain::source::SourceLifecycle;
use crate::index::scan_store::ScanStore;
use crate::index::SourceRegistry;

#[derive(Debug)]
pub enum QueryError { BadRequest, CursorStale, Internal }

#[derive(Debug, Serialize, Deserialize)]
struct Cursor {
    version: u8,
    query: String,
    revision: String,
    last_record_id: String,
}

pub fn search(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    request: SearchRequest,
) -> Result<SearchPage, QueryError> {
    let store = ScanStore::new(conn);
    let revision = store.current_index_revision().map_err(|_| QueryError::Internal)?;
    let cursor = match request.cursor() {
        Some(raw) => {
            let cursor = decode_cursor(raw).ok_or(QueryError::BadRequest)?;
            if cursor.version != 1 || cursor.query != request.query() { return Err(QueryError::BadRequest); }
            if cursor.revision != revision { return Err(QueryError::CursorStale); }
            Some(cursor)
        }
        None => None,
    };
    let mut results = store.search_records(&request, cursor.as_ref().map(|item| item.last_record_id.as_str())).map_err(|_| QueryError::Internal)?;
    let has_more = results.len() > request.limit();
    results.truncate(request.limit());
    let next_cursor = if has_more {
        results.last().map(|last| encode_cursor(&Cursor {
            version: 1,
            query: request.query().to_string(),
            revision,
            last_record_id: last.record_id().to_string(),
        }))
    } else { None };
    let empty_state = if results.is_empty() && cursor.is_none() {
        empty_state(registry, &store)?
    } else { None };
    Ok(SearchPage::new(results, next_cursor, empty_state))
}

fn empty_state(
    registry: &SourceRegistry<'_>,
    store: &ScanStore<'_>,
) -> Result<Option<SearchEmptyState>, QueryError> {
    if !store.current_index_revision().map_err(|_| QueryError::Internal)?.is_empty() { return Ok(Some(SearchEmptyState::NoMatch)); }
    let sources = registry.list().map_err(|_| QueryError::Internal)?;
    let mut unavailable = false;
    for source in sources.into_iter().filter(|source| source.lifecycle_state == SourceLifecycle::Confirmed) {
        let Some(rowid) = source.source_id.to_rowid() else { continue };
        if let Some(run) = store.latest_run(rowid).map_err(|_| QueryError::Internal)? {
            unavailable |= matches!(run.state, ScanRunState::Failed | ScanRunState::Retry);
        }
    }
    Ok(Some(if unavailable {
        SearchEmptyState::SourceUnavailable
    } else {
        SearchEmptyState::SourceNotIndexed
    }))
}

fn encode_cursor(cursor: &Cursor) -> String {
    let json = serde_json::to_vec(cursor).expect("cursor DTO serialization is total");
    let mut encoded = String::with_capacity(3 + json.len() * 2);
    encoded.push_str("v1.");
    for byte in json { encoded.push_str(&format!("{byte:02x}")); }
    encoded
}

fn decode_cursor(raw: &str) -> Option<Cursor> {
    let hex = raw.strip_prefix("v1.")?;
    if hex.is_empty() || hex.len() % 2 != 0 || raw.len() > MAX_CURSOR_BYTES { return None; }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks_exact(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        bytes.push(((hi << 4) | lo) as u8);
    }
    let cursor: Cursor = serde_json::from_slice(&bytes).ok()?;
    if cursor.query.is_empty()
        || cursor.query.len() > crate::domain::query::MAX_QUERY_BYTES
        || cursor.revision.is_empty()
        || cursor.revision.len() > 64
        || cursor.last_record_id.is_empty()
        || cursor.last_record_id.len() > 512
    {
        return None;
    }
    Some(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_round_trip_is_opaque_and_versioned() {
        let cursor = Cursor { version: 1, query: "记忆".into(), revision: "bc93e71f".into(), last_record_id: "rec_2".into() };
        let encoded = encode_cursor(&cursor);
        assert!(!encoded.contains("记忆"));
        assert_eq!(decode_cursor(&encoded).unwrap().query, "记忆");
    }
}

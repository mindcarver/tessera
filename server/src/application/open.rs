//! Server-side "open original location" orchestration.

use std::io;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use rusqlite::Connection;

use crate::domain::open::{OpenRequest, OpenResult};
use crate::domain::source::SourceId;
use crate::index::scan_store::ScanStore;
use crate::policy;

type OpenPathFn = fn(&Path) -> io::Result<()>;

static OPEN_PATH: OnceLock<Mutex<OpenPathFn>> = OnceLock::new();

#[derive(Debug)]
pub enum OpenError {
    RecordNotFound,
    OpenFailed { source_id: Option<SourceId> },
    Internal,
}

pub fn open_original_location(
    conn: &Connection,
    request: OpenRequest,
) -> Result<OpenResult, OpenError> {
    let store = ScanStore::new(conn);
    let target = store
        .open_target_for_record(request.record_id())
        .map_err(|_| OpenError::Internal)?
        .ok_or(OpenError::RecordNotFound)?;
    let source_id = target.source_id.clone();
    let target_path =
        policy::path_from_file_uri(&target.native_locator).map_err(|_| OpenError::OpenFailed {
            source_id: Some(source_id.clone()),
        })?;
    let target_path =
        policy::canonical_target_within_root(Path::new(&target.normalized_root_path), &target_path)
            .map_err(|_| OpenError::OpenFailed {
                source_id: Some(source_id.clone()),
            })?;
    invoke_open_path(&target_path).map_err(|_| OpenError::OpenFailed {
        source_id: Some(source_id.clone()),
    })?;
    Ok(OpenResult::new(target.record_id, source_id))
}

pub fn set_open_path_for_tests(open_path: OpenPathFn) {
    *open_path_slot().lock().expect("open path lock") = open_path;
}

pub fn reset_open_path_for_tests() {
    *open_path_slot().lock().expect("open path lock") = system_open_path;
}

fn invoke_open_path(path: &Path) -> io::Result<()> {
    let open_path = *open_path_slot()
        .lock()
        .map_err(|_| io::Error::other("open path lock poisoned"))?;
    open_path(path)
}

fn open_path_slot() -> &'static Mutex<OpenPathFn> {
    OPEN_PATH.get_or_init(|| Mutex::new(system_open_path))
}

fn system_open_path(path: &Path) -> io::Result<()> {
    open::that(path).map_err(|err| io::Error::other(err.to_string()))
}

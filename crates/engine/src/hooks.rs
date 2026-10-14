//! General hooks modules register with the engine (no module-specific logic here).

use std::sync::Mutex;

use crate::{CmdError, Session};

static ON_DOCUMENT_LOADED: Mutex<Vec<fn(&mut Session)>> = Mutex::new(Vec::new());
static ON_UPDATE_FIELDS: Mutex<Vec<fn(&mut Session) -> Result<(), CmdError>>> = Mutex::new(Vec::new());

/// Run after [`Session::set_document`] replaces the document (open, new, paste whole doc…).
pub fn register_on_document_loaded(f: fn(&mut Session)) {
    let mut hooks = ON_DOCUMENT_LOADED.lock().unwrap_or_else(|e| e.into_inner());
    hooks.push(f);
}

pub(crate) fn run_on_document_loaded(session: &mut Session) {
    let hooks = ON_DOCUMENT_LOADED.lock().unwrap_or_else(|e| e.into_inner());
    for f in hooks.iter() {
        f(session);
    }
}

/// Run at the end of [`crate::cmd::references::update_fields`] so optional modules can refresh
/// their own field results (dates and page numbers are handled in the engine).
pub fn register_on_update_fields(f: fn(&mut Session) -> Result<(), CmdError>) {
    let mut hooks = ON_UPDATE_FIELDS.lock().unwrap_or_else(|e| e.into_inner());
    hooks.push(f);
}

pub(crate) fn run_on_update_fields(session: &mut Session) -> Result<(), CmdError> {
    let hooks = ON_UPDATE_FIELDS.lock().unwrap_or_else(|e| e.into_inner());
    for f in hooks.iter() {
        f(session)?;
    }
    Ok(())
}

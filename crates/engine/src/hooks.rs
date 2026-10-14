//! General hooks modules register with the engine (no module-specific logic here).

use std::sync::Mutex;

use crate::{CmdError, Session};

static ON_DOCUMENT_LOADED: Mutex<Vec<fn(&mut Session)>> = Mutex::new(Vec::new());
static ON_UPDATE_FIELDS: Mutex<Vec<fn(&mut Session) -> Result<(), CmdError>>> = Mutex::new(Vec::new());
static AFTER_FIELD_UPDATE: Mutex<Vec<fn(&mut Session) -> Result<(), CmdError>>> = Mutex::new(Vec::new());

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

static ON_TEXT_AUTOFORMAT: Mutex<Vec<fn(&mut Session, &str) -> Result<bool, CmdError>>> = Mutex::new(Vec::new());

/// Run from [`crate::cmd::text::insert`] after built-in AutoFormat on a single typed character.
/// Return `Ok(true)` when the hook changed the document (undo is handled there).
pub fn register_on_text_autoformat(f: fn(&mut Session, &str) -> Result<bool, CmdError>) {
    let mut hooks = ON_TEXT_AUTOFORMAT.lock().unwrap_or_else(|e| e.into_inner());
    hooks.push(f);
}

pub(crate) fn run_on_text_autoformat(session: &mut Session, text: &str) -> Result<bool, CmdError> {
    let hooks = ON_TEXT_AUTOFORMAT.lock().unwrap_or_else(|e| e.into_inner());
    for f in hooks.iter() {
        if f(session, text)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Serialises tests that register hooks: the registry is process-wide, so a test that needs to
/// see only its own hooks takes this lock and empties the registry around itself.
#[cfg(test)]
pub(crate) static HOOK_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Empty every hook registry. Tests use it (under [`HOOK_TEST_LOCK`]) so a hook one test registers
/// can never run for another test's session; production never unregisters.
#[cfg(test)]
pub(crate) fn reset_for_test() {
    ON_DOCUMENT_LOADED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    ON_TEXT_AUTOFORMAT.lock().unwrap_or_else(|e| e.into_inner()).clear();
    ON_UPDATE_FIELDS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    AFTER_FIELD_UPDATE.lock().unwrap_or_else(|e| e.into_inner()).clear();
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

/// Run after [`crate::cmd::references::update_fields`] has recomputed field results. A module
/// whose generated lines depend on those results (an index of authorities, say) registers one;
/// the engine runs them in registration order and stops at the first error. The hook sees a
/// session whose field results are already current; it holds no module-specific logic itself.
pub fn register_after_field_update(f: fn(&mut Session) -> Result<(), CmdError>) {
    let mut hooks = AFTER_FIELD_UPDATE.lock().unwrap_or_else(|e| e.into_inner());
    hooks.push(f);
}

pub(crate) fn run_after_field_update(session: &mut Session) -> Result<(), CmdError> {
    let hooks = AFTER_FIELD_UPDATE.lock().unwrap_or_else(|e| e.into_inner());
    for f in hooks.iter() {
        f(session)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static RUNS: AtomicUsize = AtomicUsize::new(0);
    // A thread-local so this test's failing hook never affects another test's session.
    thread_local! {
        static FAIL: Cell<bool> = const { Cell::new(false) };
    }

    fn observe(_: &mut Session) -> Result<(), CmdError> {
        RUNS.fetch_add(1, Ordering::SeqCst);
        if FAIL.with(Cell::get) {
            return Err(CmdError::Failed("hook failed".into()));
        }
        Ok(())
    }

    fn register_once() {
        static REGISTERED: OnceLock<()> = OnceLock::new();
        if REGISTERED.set(()).is_ok() {
            register_after_field_update(observe);
        }
    }

    #[test]
    fn after_field_update_hooks_run_and_errors_propagate() {
        register_once();
        let mut s = Session::new(wordcraft_doc::Document::from_text("one\ntwo"));
        let before = RUNS.load(Ordering::SeqCst);
        run_after_field_update(&mut s).expect("the hook returns ok");
        assert!(RUNS.load(Ordering::SeqCst) > before, "the registered hook ran");
        FAIL.with(|f| f.set(true));
        assert!(matches!(run_after_field_update(&mut s), Err(CmdError::Failed(m)) if m == "hook failed"), "a hook error stops the run");
        FAIL.with(|f| f.set(false));
    }
}

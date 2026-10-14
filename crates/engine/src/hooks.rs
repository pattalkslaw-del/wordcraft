//! General hooks modules register with the engine (no module-specific logic here).

use std::sync::Mutex;

use crate::{CmdError, Session};

static ON_DOCUMENT_LOADED: Mutex<Vec<fn(&mut Session)>> = Mutex::new(Vec::new());
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

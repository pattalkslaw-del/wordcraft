//! Engine hook registration (document loaded, typing AutoFormat).

use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use wordcraft_doc::Document;

use crate::Session;
use crate::hooks::{register_on_document_loaded, register_on_text_autoformat, run_on_text_autoformat};

#[test]
fn document_loaded_hooks_run_after_set_document() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    register_on_document_loaded(|_| {
        CALLS.fetch_add(1, Ordering::SeqCst);
    });
    let before = CALLS.load(Ordering::SeqCst);
    let mut s = Session::new(Document::new());
    s.set_document(Document::new());
    assert!(CALLS.load(Ordering::SeqCst) > before, "set_document runs document-loaded hooks");
}

#[test]
fn text_autoformat_hooks_stop_after_the_first_change() {
    static ORDER: AtomicUsize = AtomicUsize::new(0);
    register_on_text_autoformat(|_, _| {
        ORDER.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    });
    register_on_text_autoformat(|_, _| {
        ORDER.fetch_add(10, Ordering::SeqCst);
        Ok(false)
    });
    let mut s = Session::new(Document::new());
    let before = ORDER.load(Ordering::SeqCst);
    assert!(run_on_text_autoformat(&mut s, "1").unwrap());
    let after = ORDER.load(Ordering::SeqCst);
    assert_eq!(after, before + 1, "only the first hook that returns true should run");
}

#[test]
fn text_autoformat_hook_runs_from_text_insert() {
    static HIT: AtomicUsize = AtomicUsize::new(0);
    register_on_text_autoformat(|_, text| {
        if text == "z" {
            HIT.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        } else {
            Ok(false)
        }
    });
    let before = HIT.load(Ordering::SeqCst);
    let mut s = Session::new(Document::new());
    s.run("text.insert", &json!({"text": "z"})).unwrap();
    assert_eq!(HIT.load(Ordering::SeqCst), before + 1);
}

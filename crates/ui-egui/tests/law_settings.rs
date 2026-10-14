//! File › Options › LawCraft: profiles list, edit, import/export, approve.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static LAW_TEST_ID: AtomicU64 = AtomicU64::new(0);

fn unique_id(prefix: &str) -> String {
    format!("{}-{}", prefix, LAW_TEST_ID.fetch_add(1, Ordering::Relaxed))
}

/// The law profile store is process-wide; tests in this file run one at a time.
static LAW_SETTINGS_LOCK: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    LAW_SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};

use egui_kittest::kittest::Queryable;
use serde_json::json;
use wordcraft_doc::Document;
use wordcraft_engine::Session;
use wordcraft_law::{MODULE, install};
use wordcraft_ui_egui::control::{ControlRequest, Outcome, handle};
use wordcraft_ui_egui::file_dialogs::{FileDialogAnswer, FileDialogRequest};
use wordcraft_ui_egui::{Services, WordApp};

type Shown = Rc<RefCell<Vec<(FileDialogRequest, Sender<FileDialogAnswer>)>>>;

fn law_app() -> WordApp {
    let mut s = Session::new(Document::new());
    s.prefs.modules.insert(MODULE.into(), true);
    install(&mut s);
    WordApp::new(s, Services::default())
}

fn async_app() -> (WordApp, Shown) {
    let shown: Shown = Rc::default();
    let log = shown.clone();
    let services = Services {
        file_dialog: Some(Box::new(move |req| {
            let (tx, rx) = channel();
            log.borrow_mut().push((req, tx));
            rx
        })),
        ..Default::default()
    };
    let mut s = Session::new(Document::new());
    s.prefs.modules.insert(MODULE.into(), true);
    install(&mut s);
    (WordApp::new(s, services), shown)
}

fn answer(shown: &Shown, picked: Option<&str>) {
    let (_, tx) = shown.borrow_mut().pop().unwrap();
    tx.send(Ok(picked.map(str::to_string))).unwrap();
}

fn open_law_options(a: &mut WordApp) {
    a.run("file.options", json!({"pane": "law"})).unwrap();
    assert!(a.ui.backstage);
}

fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    )
}

#[test]
fn law_options_lists_bundled_profiles_with_verification_badges() {
    let _lock = serial();
    let mut a = law_app();
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(6);
    assert!(h.query_by_label("Missouri appellate brief").is_some());
    assert!(h.query_by_label("Federal appellate brief").is_some());
    assert!(h.query_by_label("Missouri").is_some());
}

#[test]
fn editing_a_profile_marks_it_unverified() {
    let _lock = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-edit-unverified");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Edit unverified test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [{"name": "Normal"}],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(6);
    h.state_mut().law.selected = Some(id.to_string());
    h.state_mut().law.doc_type = "brief".to_string();
    h.run_steps(4);
    h.state_mut().run("law.profiles.update", json!({"id": id, "name": "Edit unverified test (changed)"})).unwrap();
    h.run_steps(2);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(after["verified"], json!(false));
}

#[test]
fn law_profile_import_and_export_through_file_dialogs() {
    let _lock = serial();
    let dir = std::env::temp_dir().join(format!("wc-ui-law-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("export.json");

    let (mut a, shown) = async_app();
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(6);
    h.get_by_label("Missouri appellate brief").click();
    h.run_steps(4);
    h.get_by_label("Export…").click();
    h.run_steps(2);
    assert!(matches!(shown.borrow().last().map(|(r, _)| r.clone()), Some(FileDialogRequest::Save { .. })));
    answer(&shown, Some(&path.to_string_lossy()));
    h.run_steps(4);
    assert!(path.is_file());

    h.get_by_label("Import…").click();
    h.run_steps(2);
    answer(&shown, Some(&path.to_string_lossy()));
    h.run_steps(4);
    let imported = h.state_mut().run("law.profiles.get", json!({"id": "missouri-appellate-brief"})).unwrap();
    assert_eq!(imported["name"], json!("Missouri appellate brief"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn approve_profile_in_ui_and_block_automation() {
    let _lock = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-approve-test");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Approve UI test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [{"name": "Normal"}],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(a.run("law.profiles.get", json!({"id": id})).unwrap()["verified"], json!(false));
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(8);
    h.get_by_label("Approve UI test").click();
    h.run_steps(4);
    let before = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(before["verified"], json!(false));
    h.get_by_label("Approve profile…").click();
    h.run_steps(4);
    assert!(h.state().law.confirm_approve, "Approve profile opens the confirmation step");
    h.get_by_label("Approve").click();
    h.run_steps(4);
    let p = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(p["verified"], json!(true));

    let ctx = egui::Context::default();
    let (req, _rx) = ControlRequest::new("engine.execute", json!({"command": "law.profiles.approve", "params": {"id": id}}));
    let Outcome::Done(v) = handle(h.state_mut(), &ctx, &req) else { panic!("screenshot") };
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap_or("").contains("outside automation"));
}

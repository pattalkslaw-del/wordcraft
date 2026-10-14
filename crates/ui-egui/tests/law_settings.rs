//! File › Options › LawCraft: profiles list, edit, import/export, approve.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static LAW_TEST_ID: AtomicU64 = AtomicU64::new(0);

fn unique_id(prefix: &str) -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{}-{}-{}", prefix, std::process::id(), LAW_TEST_ID.fetch_add(1, Ordering::Relaxed), n)
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

fn plain_app() -> WordApp {
    WordApp::new(Session::new(Document::new()), Services::default())
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
    assert!(h.query_by_label("Verified").is_some());
    assert!(h.query_by_label("missouri-appellate-brief").is_some());
}

#[test]
fn law_module_toggle_uses_file_options_and_shows_law_pane() {
    let _lock = serial();
    let mut a = plain_app();
    assert!(!a.session.module_enabled(MODULE));
    a.run("file.options", json!({"pane": "addins"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert!(h.query_by_label("LawCraft").is_none(), "LawCraft pane hidden until the module is on");
    h.get_by_label("LawCraft (court profiles and legal formatting)").click();
    h.run_steps(6);
    assert!(h.state().session.module_enabled(MODULE));
    assert_eq!(h.state().ui.options_pane, "law");
    assert!(h.query_by_label("Court profiles").is_some());
    let opts = h.state_mut().run("file.options", json!({"modules": {"law": false}})).unwrap();
    assert_eq!(opts["lawEnabled"], json!(false));
    h.run_steps(2);
    assert!(!h.state().session.module_enabled(MODULE));
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
                    "sections": [{"id": "cover", "name": "Cover"}, {"id": "body", "name": "Body"}],
                    "checks": [{"kind": "requiredSections", "ids": ["cover"], "source": "Test"}],
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
    h.get_by_label("Edit unverified test").click();
    h.run_steps(6);
    h.get_by_label("Add check").click();
    h.run_steps(4);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(after["verified"], json!(false));
    assert!(after["docs"]["brief"]["checks"].as_array().is_some_and(|c| c.len() >= 2), "Add check saves through the editor");
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
    assert_eq!(h.state().law.doc_type, "appendix", "import picks the profile's first document type");
    let section_count = imported["docs"]["appendix"]["sections"].as_array().map(|a| a.len()).unwrap_or(0);
    assert!(section_count > 0);
    h.run_steps(4);
    assert!(h.query_by_label("Sections").is_some(), "editor shows the selected document type's sections");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn editing_a_check_source_across_frames_saves_on_blur() {
    let _lock = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-source-edit");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Source edit test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "noHyperlinks", "source": "Old"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(6);
    h.get_by_label("Source edit test").click();
    h.run_steps(6);

    // Type one character per frame: egui rebuilds the widget buffer every frame, so the draft has
    // to be kept between frames or the characters would be lost before the field can save.
    h.get_by_label("Source:").focus();
    h.run_steps(2);
    for ch in "New".chars() {
        h.get_by_label("Source:").type_text(&ch.to_string());
        h.run_steps(1);
    }
    let shown = h.get_by_label("Source:").value().unwrap_or_default();
    assert!(shown.ends_with("New"), "the field keeps every character typed, not just the last: {shown:?}");

    // Blurring commits through law.profiles.update.
    h.get_by_label("Add check").focus();
    h.run_steps(2);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    let saved = after["docs"]["brief"]["checks"][0]["source"].as_str().unwrap_or("");
    assert_ne!(saved, "Old", "the source edit is saved, not discarded");
    assert!(saved.ends_with("New"), "saved source {saved:?}");
}

#[test]
fn editing_a_section_name_across_frames_saves_on_blur() {
    let _lock = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-section-name");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Section edit test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [],
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
    h.get_by_label("Section edit test").click();
    h.run_steps(6);

    h.get_by_label("1.").focus();
    h.run_steps(2);
    for ch in " Page".chars() {
        h.get_by_label("1.").type_text(&ch.to_string());
        h.run_steps(1);
    }
    h.get_by_label("Add check").focus();
    h.run_steps(2);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(after["docs"]["brief"]["sections"][0]["name"], json!("Cover Page"));
    assert_eq!(after["verified"], json!(false), "editing the profile makes it unverified again");
}

#[test]
fn editing_a_style_font_across_frames_keeps_spaces() {
    let _lock = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-style-font");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Style font test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [{"name": "Norm1"}],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    a.law.selected = Some(id.clone());
    a.law.doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(6);
    h.get_by_label("Norm1").click();
    h.run_steps(2);

    // One character per frame, spaces included: the untrimmed draft must survive between frames.
    h.get_by_label("Font:").focus();
    h.run_steps(2);
    for ch in "Times New Roman".chars() {
        h.get_by_label("Font:").type_text(&ch.to_string());
        h.run_steps(1);
    }
    let shown = h.get_by_label("Font:").value().unwrap_or_default();
    assert_eq!(shown, "Times New Roman", "the style font field keeps the spaces typed: {shown:?}");

    h.get_by_label("Add check").focus();
    h.run_steps(2);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(after["docs"]["brief"]["styles"][0]["chr"]["font"], json!("Times New Roman"));
    assert_eq!(after["verified"], json!(false), "editing a style makes the profile unverified");
}

#[test]
fn editing_a_check_font_across_frames_keeps_spaces() {
    let _lock = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-check-font");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Check font test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "minFontSize", "pt": 13.0, "font": "", "source": "Test"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    a.law.selected = Some(id.clone());
    a.law.doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(6);

    h.get_by_label("Font:").focus();
    h.run_steps(2);
    for ch in "Times New Roman".chars() {
        h.get_by_label("Font:").type_text(&ch.to_string());
        h.run_steps(1);
    }
    let shown = h.get_by_label("Font:").value().unwrap_or_default();
    assert_eq!(shown, "Times New Roman", "the check font field keeps the spaces typed: {shown:?}");

    h.get_by_label("Add check").focus();
    h.run_steps(2);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(after["docs"]["brief"]["checks"][0]["font"], json!("Times New Roman"));
    assert_eq!(after["verified"], json!(false));
}

#[test]
fn editing_source_and_exclusion_together_keeps_both() {
    let _lock = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-check-merge");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Check merge test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "s1", "name": "Section One"}, {"id": "s2", "name": "Section Two"}],
                    "checks": [{"kind": "wordLimit", "limit": 10000, "excludeSections": [], "source": "Old"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    a.law.selected = Some(id.clone());
    a.law.doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    h.get_by_label("Source:").focus();
    h.run_steps(2);
    for ch in "New".chars() {
        h.get_by_label("Source:").type_text(&ch.to_string());
        h.run_steps(1);
    }
    // Clicking the exclusion checkbox also blurs Source in the same frame: both edits must survive.
    h.get_by_label("s1").click();
    h.run_steps(2);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    let check = &after["docs"]["brief"]["checks"][0];
    assert!(check["source"].as_str().unwrap_or("").ends_with("New"), "the source edit is saved: {check}");
    assert_eq!(check["excludeSections"], json!(["s1"]), "the exclusion survives the source commit: {check}");
    assert_eq!(after["verified"], json!(false));
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

//! Desktop update notice (#15, #201). Checks go through `file.checkForUpdates`; this module
//! paints the notice bar and runs network/install work off the UI thread.

use std::sync::mpsc::{Receiver, TryRecvError};

use egui::{Stroke, Ui, vec2};
use serde_json::{Value, json};
use wordcraft_engine::update::{EnvReadBox, HttpGet, InstallFn, UpdatePrefs};
use wordcraft_engine::{CmdError, Session};

use crate::WordApp;
use crate::theme::{Tokens, TypeRung};

pub use wordcraft_engine::update::{INTERVAL_SECS, MAX_JSON, due};

#[derive(Default)]
pub struct UpdateCheck {
    started: bool,
    pub notice: Option<Notice>,
    worker: Option<PendingWorker>,
    /// A successful update asked to quit. The host runs the usual close path for it, so a
    /// document edited after the worker finished still gets the save prompt (#15).
    pub quit_pending: bool,
    /// A finished install needs a restart to complete (Windows MSI 3010/1641). Kept visible until
    /// acknowledged, so the instruction is not lost when the app quits to let the restart happen.
    pub reboot_notice: bool,
}

struct PendingWorker {
    rx: Receiver<WorkerDone>,
    manual: bool,
    apply: bool,
    control_reply: Option<std::sync::mpsc::Sender<Value>>,
}

struct WorkerDone {
    result: Result<Value, String>,
    prefs: UpdatePrefs,
    ui_requests: Vec<Value>,
}

struct CheckSnapshot {
    http: HttpGet,
    install: Option<InstallFn>,
    exe: Option<String>,
    env: Option<EnvReadBox>,
    prefs: UpdatePrefs,
    dirty: bool,
    now_unix: fn() -> u64,
    release_arch: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub version: String,
    pub notes: String,
    pub page_url: String,
    pub one_click: bool,
    pub verified_update: bool,
    pub external_updater: bool,
}

/// Build a notice from a command `ui_request` `update` object or a full check result.
pub fn notice_from_payload(value: &Value) -> Option<Notice> {
    if let Some(rel) = value.get("release") {
        return notice_from_fields(rel);
    }
    notice_from_fields(value)
}

fn notice_from_fields(v: &Value) -> Option<Notice> {
    let version = v.get("version").and_then(Value::as_str)?;
    Some(Notice {
        version: version.into(),
        notes: v.get("notes").and_then(Value::as_str).unwrap_or("").into(),
        page_url: v.get("pageUrl").and_then(Value::as_str).unwrap_or(wordcraft_engine::update::RELEASES_PAGE).into(),
        one_click: v.get("oneClick").and_then(Value::as_bool) == Some(true),
        verified_update: v.get("verifiedUpdate").and_then(Value::as_bool) == Some(true),
        external_updater: v.get("externalUpdater").and_then(Value::as_bool) == Some(true),
    })
}

pub fn poll(app: &mut WordApp, ctx: &egui::Context) {
    poll_worker(app);
    if app.update.worker.is_some() {
        return;
    }
    if app.update.started || app.fonts_frames < 2 {
        return;
    }
    if app.session.update_http.is_none() {
        return;
    }
    let now = now_secs();
    if !due(app.session.update_prefs.check_on_startup, app.session.update_prefs.last_check_unix, now, false) {
        return;
    }
    app.update.started = true;
    queue_check(app, json!({"startup": true}), false, ctx);
}

/// Queue `file.checkForUpdates` on a worker (network, download and installer handoff stay off the UI thread).
pub fn queue_check(app: &mut WordApp, params: Value, manual: bool, ctx: &egui::Context) {
    let _ = queue_check_inner(app, params, manual, ctx, None);
}

/// Like [`queue_check`], but completes a control-channel reply when the worker finishes.
pub fn queue_check_control(app: &mut WordApp, params: Value, ctx: &egui::Context, reply: std::sync::mpsc::Sender<Value>) -> bool {
    queue_check_inner(app, params, true, ctx, Some(reply))
}

fn queue_check_inner(
    app: &mut WordApp,
    params: Value,
    manual: bool,
    ctx: &egui::Context,
    control_reply: Option<std::sync::mpsc::Sender<Value>>,
) -> bool {
    if app.update.worker.is_some() {
        if let Some(reply) = control_reply {
            let _ = reply.send(json!({"ok": false, "error": "an update check is already running"}));
        }
        return false;
    }
    let Some(snapshot) = snapshot_from(app) else {
        if let Some(reply) = control_reply {
            let _ = reply.send(json!({"ok": false, "error": "update check is not available"}));
        }
        return false;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    let apply = params.get("apply").and_then(Value::as_bool) == Some(true);
    match std::thread::Builder::new().name("wordcraft-update".into()).spawn(move || {
        let done = run_check(snapshot, &params);
        let _ = tx.send(done);
        ctx.request_repaint();
    }) {
        Ok(_) => {
            app.update.worker = Some(PendingWorker { rx, manual, apply, control_reply });
            true
        }
        Err(_) => {
            let msg = "could not start the update check";
            if let Some(reply) = control_reply {
                let _ = reply.send(json!({"ok": false, "error": msg}));
            } else if manual {
                crate::backstage::report_update_check(app, Err(msg.into()));
            } else {
                log::warn!("{msg}");
            }
            false
        }
    }
}

fn poll_worker(app: &mut WordApp) {
    let Some(worker) = app.update.worker.as_ref() else { return };
    let msg = match worker.rx.try_recv() {
        Ok(done) => done,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => WorkerDone {
            result: Err("the update check stopped unexpectedly".into()),
            prefs: app.session.update_prefs.clone(),
            ui_requests: Vec::new(),
        },
    };
    let (manual, apply, control_reply) =
        app.update.worker.as_ref().map(|w| (w.manual, w.apply, w.control_reply.clone())).unwrap_or((false, false, None));
    app.update.worker = None;
    let result = apply_worker_result(app, msg, manual, apply);
    if let Some(reply) = control_reply {
        let payload = match result {
            Ok(v) => json!({"ok": true, "result": v}),
            Err(e) => json!({"ok": false, "error": e}),
        };
        let _ = reply.send(payload);
    }
}

fn snapshot_from(app: &WordApp) -> Option<CheckSnapshot> {
    let http = app.session.update_http.clone()?;
    Some(CheckSnapshot {
        http,
        install: app.session.update_install.clone(),
        exe: app.session.update_exe.clone(),
        env: app.session.update_env.clone(),
        prefs: app.session.update_prefs.clone(),
        dirty: app.session.dirty,
        now_unix: app.session.update_now_unix,
        release_arch: app.session.update_release_arch.clone(),
    })
}

fn run_check(snapshot: CheckSnapshot, params: &Value) -> WorkerDone {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.update_http = Some(snapshot.http);
    s.update_install = snapshot.install;
    s.update_exe = snapshot.exe;
    s.update_env = snapshot.env;
    s.update_prefs = snapshot.prefs;
    s.update_now_unix = snapshot.now_unix;
    s.update_release_arch = snapshot.release_arch;
    s.dirty = snapshot.dirty;
    let result = s.run("file.checkForUpdates", params).map_err(|e: CmdError| e.to_string());
    WorkerDone { result, prefs: s.update_prefs, ui_requests: s.ui_requests }
}

fn apply_worker_result(app: &mut WordApp, done: WorkerDone, manual: bool, apply: bool) -> Result<Value, String> {
    app.session.update_prefs.last_check_unix = done.prefs.last_check_unix;
    app.ui.update_prefs.last_check_unix = done.prefs.last_check_unix;
    let mut ui_requests = done.ui_requests;
    if apply {
        // The installer's quit is not confirmed against the document as it is now: drop the
        // direct quit request and let the host's close path ask about edits made after the
        // worker finished. `apply_install_result` reports when the close may go ahead.
        ui_requests.retain(|r| r.get("quit").and_then(Value::as_bool) != Some(true));
    }
    app.apply_ui_requests(ui_requests);
    match done.result {
        Ok(v) => {
            if apply {
                if apply_install_result(app, Ok(v.clone())) {
                    app.update.quit_pending = true;
                }
                Ok(v)
            } else if manual {
                crate::backstage::report_update_check(app, Ok(v.clone()));
                Ok(v)
            } else {
                Ok(v)
            }
        }
        Err(e) => {
            if apply {
                let _ = apply_install_result(app, Err(e.clone()));
            } else if manual {
                crate::backstage::report_update_check(app, Err(e.clone()));
            } else {
                log::info!("startup update check failed: {e}");
            }
            Err(e)
        }
    }
}

fn should_quit_after_successful_apply(v: &Value) -> bool {
    if v.get("status").and_then(Value::as_str) != Some("available") {
        return false;
    }
    v.get("release").and_then(|r| r.get("oneClick")).and_then(Value::as_bool) == Some(true)
}

fn apply_install_result(app: &mut WordApp, v: Result<Value, String>) -> bool {
    match v {
        Ok(v) => {
            if v.get("status") == Some(&Value::String("error".into())) {
                let key = v.get("error").and_then(Value::as_str).unwrap_or("");
                let text = if key.is_empty() { tl!("The update could not be installed.").to_string() } else { tl!(key).to_string() };
                if let Some(detail) = v.get("errorDetail").and_then(Value::as_str) {
                    log::warn!("update failed: {detail}");
                }
                app.status(text);
                false
            } else if app.session.dirty {
                app.status(tl!("Save or discard unsaved changes before updating."));
                false
            } else {
                // Only MSI / .deb one-click handoffs quit; macOS opens the `.dmg` in Finder.
                should_quit_after_successful_apply(&v)
            }
        }
        Err(e) => {
            log::warn!("update failed: {e}");
            app.status(tl!(wordcraft_engine::update::MSG_START_FAILED));
            false
        }
    }
}

/// True when this build can run update checks off the UI thread.
pub fn can_run_off_thread(app: &WordApp) -> bool {
    app.session.update_http.is_some()
}

pub fn worker_busy(app: &WordApp) -> bool {
    app.update.worker.is_some()
}

/// Manual check or command-driven notice (`file.checkForUpdates` ui_request).
pub fn apply_command_result(app: &mut WordApp, value: &Value) {
    if let Some(n) = notice_from_payload(value) {
        app.update.notice = Some(n);
    }
}

/// The restart instruction after a finished install: a durable bar, not a status message that
/// vanishes before the app quits. Dismissed only when the user acknowledges it.
fn reboot_bar(app: &mut WordApp, ui: &mut Ui) {
    if !app.update.reboot_notice {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let mut acknowledged = false;
    egui::Panel::top("update_reboot_notice")
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(12, 6)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                let text = tl!("WordCraft was updated. Restart the computer to finish the update.");
                ui.label(egui::RichText::new(text).font(TypeRung::Control.medium()).color(t.text));
                if ui.button(tl!("OK")).clicked() {
                    acknowledged = true;
                }
            });
        });
    if acknowledged {
        app.update.reboot_notice = false;
    }
}

pub fn bar(app: &mut WordApp, ui: &mut Ui) {
    reboot_bar(app, ui);
    let Some(notice) = app.update.notice.clone() else { return };
    let t = Tokens::get(ui.ctx());
    let mut close = false;
    let ctx = ui.ctx().clone();
    egui::Panel::top("update_notice")
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(12, 6)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                    let text = crate::i18n::fmt(tl!("WordCraft {version} is available."), &[("version", &notice.version)]);
                    ui.label(egui::RichText::new(text).font(TypeRung::Control.medium()).color(t.text));
                    if (notice.one_click || notice.verified_update) && !notice.external_updater {
                        if ui.button(tl!("Update")).clicked() {
                            if app.session.dirty {
                                let name = app
                                    .session
                                    .path
                                    .as_deref()
                                    .map(|p| {
                                        p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string_lossy().to_string())
                                    })
                                    .unwrap_or_else(|| "Document".into());
                                app.dialog = Some(crate::dialogs::Dialog::SaveChanges {
                                    name,
                                    then: "file.checkForUpdates".into(),
                                    params: json!({"force": true, "apply": true}),
                                    document: app.session.document_id(),
                                });
                            } else if app.update.worker.is_none() {
                                queue_check(app, json!({"force": true, "apply": true}), false, &ctx);
                            }
                        }
                    } else if ui.button(tl!("Download")).clicked() {
                        app.canvas.open_url = Some(notice.page_url.clone());
                        close = true;
                    }
                    if ui.button(tl!("Skip this version")).clicked() {
                        let _ = app.run("file.updatePrefs", json!({"skipVersion": notice.version}));
                        close = true;
                    }
                    if ui.button(tl!("Later")).clicked() {
                        close = true;
                    }
                });
                if notice.external_updater {
                    ui.label(egui::RichText::new(tl!("Updates for this install are provided by Flatpak or AppImage, not from here.")).small().weak());
                }
                if !notice.notes.is_empty() {
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().max_height(80.0).show(ui, |ui| {
                        ui.label(egui::RichText::new(&notice.notes).small());
                    });
                }
            });
        });
    if app.update.worker.is_none() && app.quit_requested {
        close = true;
    }
    if close {
        app.update.notice = None;
    }
}

pub fn maybe_startup_prompt(app: &mut WordApp) {
    if !app.session.update_prefs.asked_startup {
        app.show_update_startup_prompt = true;
    }
}

pub fn startup_prompt(app: &mut WordApp, ui: &mut Ui, open: &mut bool) {
    egui::Window::new(tl!("Check for updates?")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
        ui.ctx(),
        |ui| {
            ui.label(tl!("Once a day, WordCraft can ask GitHub whether a newer WordCraft release is out. Nothing else is sent."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("Yes")).clicked() {
                    let _ = app.run("file.updatePrefs", json!({"checkOnStartup": true, "askedStartup": true}));
                    *open = false;
                }
                if ui.button(tl!("No")).clicked() {
                    let _ = app.run("file.updatePrefs", json!({"checkOnStartup": false, "askedStartup": true}));
                    *open = false;
                }
            });
        },
    );
}

fn now_secs() -> u64 {
    let ms = crate::now_ms();
    if ms.is_finite() && ms > 0.0 { (ms / 1000.0) as u64 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_request_update_payload_builds_notice() {
        let up = json!({
            "version": "9.0.0",
            "notes": "Hi",
            "pageUrl": "https://github.com/storytold/wordcraft/releases/tag/v9.0.0",
            "oneClick": true,
            "externalUpdater": false,
        });
        let n = notice_from_payload(&up).unwrap();
        assert_eq!(n.version, "9.0.0");
        assert!(n.one_click);
    }

    #[test]
    fn worker_apply_with_dirty_document_does_not_request_quit() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.session.dirty = true;
        let done = WorkerDone { result: Ok(json!({"status": "available"})), prefs: UpdatePrefs::default(), ui_requests: vec![json!({"quit": true})] };
        let _ = apply_worker_result(&mut app, done, false, true);
        assert!(!app.quit_requested, "edits after the save prompt must block quitting");
        assert!(!app.update.quit_pending, "and no quit is left pending");
    }

    /// A successful update sets the quit, but editing remains possible before the host acts on
    /// it: those edits must still be protected by the save prompt (#15).
    #[test]
    fn update_quit_after_a_new_edit_still_prompts_and_cancel_keeps_the_document() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let done = WorkerDone {
            result: Ok(json!({"status": "available", "release": {"oneClick": true}})),
            prefs: UpdatePrefs::default(),
            ui_requests: vec![json!({"quit": true})],
        };
        let _ = apply_worker_result(&mut app, done, false, true);
        assert!(app.update.quit_pending, "a successful one-click update asks to quit");
        assert!(!app.quit_requested, "the quit goes through the close path, not straight to quit");

        // The user types before the host acts on the quit.
        app.session.dirty = true;
        assert!(!app.update_quit_requested(), "unsaved edits block the close");
        assert!(matches!(app.dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })), "the save prompt is shown");

        // Cancelling keeps the document open; nothing is lost.
        app.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert!(!app.quit_requested, "cancelling keeps the document open");
        assert!(!app.update.quit_pending, "the pending update quit was consumed");
    }

    /// A clean document quits once the update finishes, through the same path.
    #[test]
    fn update_quit_on_a_clean_document_closes() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let done = WorkerDone {
            result: Ok(json!({"status": "available", "release": {"oneClick": true}})),
            prefs: UpdatePrefs::default(),
            ui_requests: vec![json!({"quit": true})],
        };
        let _ = apply_worker_result(&mut app, done, false, true);
        assert!(app.update_quit_requested(), "a clean document may close");
        assert!(!app.update.quit_pending, "the pending quit was consumed");
        // The host asks the window to close; the next frame's close path allows it.
        assert!(app.close_requested(), "the close goes ahead");
        assert!(!app.quit_requested, "no user quit was recorded");
    }

    /// A re-check that finds no newer release (or errors) must not quit.
    #[test]
    fn update_result_without_a_handoff_does_not_quit() {
        for result in [json!({"status": "current"}), json!({"status": "throttled"}), json!({"status": "error", "error": "x"})] {
            let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
            let done = WorkerDone { result: Ok(result.clone()), prefs: UpdatePrefs::default(), ui_requests: vec![json!({"quit": true})] };
            let _ = apply_worker_result(&mut app, done, false, true);
            assert!(!app.update.quit_pending, "{result} must not quit");
            assert!(!app.quit_requested, "{result} must not quit");
        }
    }

    #[test]
    fn command_result_release_builds_notice() {
        let r = json!({
            "status": "available",
            "release": {
                "version": "8.0.0",
                "notes": "",
                "pageUrl": "https://github.com/storytold/wordcraft/releases/latest",
                "oneClick": false,
                "externalUpdater": true,
            }
        });
        let n = notice_from_payload(&r).unwrap();
        assert_eq!(n.version, "8.0.0");
        assert!(n.external_updater);
    }

    /// The restart instruction survives the quit request (a status message would vanish first), and
    /// clearing it is explicit, as the banner's OK button does (#15).
    #[test]
    fn reboot_notice_is_durable_until_acknowledged() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.apply_ui_requests(vec![json!({"quit": true}), json!({"updateReboot": true})]);
        assert!(app.update.reboot_notice, "the restart instruction is kept, not a transient status");
        assert!(app.quit_requested, "the update still asks to quit through the close path");
        app.update.reboot_notice = false;
        assert!(!app.update.reboot_notice, "acknowledging the banner clears it");
    }

    /// macOS verified download: drag instruction, no quit (#15 round 12).
    #[test]
    fn macos_dmg_handoff_shows_instruction_without_quit() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let done = WorkerDone {
            result: Ok(json!({
                "status": "available",
                "release": {
                    "version": "9.0.0",
                    "oneClick": false,
                    "verifiedUpdate": true,
                }
            })),
            prefs: UpdatePrefs::default(),
            ui_requests: vec![json!({"updateMacDmg": true})],
        };
        let _ = apply_worker_result(&mut app, done, false, true);
        assert!(!app.update.quit_pending, "macOS must not quit after opening the dmg");
        assert!(!app.quit_requested);
        let msg = app.status_msg.as_ref().map(|(s, _)| s.as_str()).unwrap_or("");
        assert!(msg.contains("Drag WordCraft into Applications"), "expected drag instruction in status, got {msg:?}");
    }

    #[test]
    fn reboot_notice_defers_update_quit_until_acknowledged() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.update.quit_pending = true;
        app.apply_ui_requests(vec![json!({"updateReboot": true})]);
        assert!(app.update.reboot_notice);
        assert!(!app.update_quit_requested(), "do not close while the restart banner is visible");
        assert!(app.update.quit_pending, "the quit stays pending until after acknowledgement");
        app.update.reboot_notice = false;
        assert!(app.update_quit_requested(), "after OK, the close may proceed");
    }
}

//! Desktop update notice (#15, #201). Checks go through `file.checkForUpdates`; this module
//! polls a background fetch on startup and paints the notice bar.

use std::sync::mpsc::Receiver;

use egui::{Stroke, Ui, vec2};
use serde_json::{Value, json};

use crate::WordApp;
use crate::theme::{Tokens, TypeRung};

pub use wordcraft_engine::update::{INTERVAL_SECS, MAX_JSON, due, offer, parse_latest_release, parse_version};

/// Fetches the latest-release JSON. The host caps size and time.
pub type Fetch = std::sync::Arc<dyn Fn() -> Result<Vec<u8>, String> + Send + Sync>;

#[derive(Default)]
pub struct UpdateCheck {
    started: bool,
    rx: Option<Receiver<Result<Vec<u8>, String>>>,
    pub notice: Option<Notice>,
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub version: String,
    pub notes: String,
    pub page_url: String,
    pub one_click: bool,
    pub external_updater: bool,
}

pub fn poll(app: &mut WordApp, ctx: &egui::Context) {
    collect_fetch(app);
    if app.update.started || app.fonts_frames < 2 {
        return;
    }
    let now = now_secs();
    if !due(app.session.update_prefs.check_on_startup, app.session.update_prefs.last_check_unix, now, false) {
        return;
    }
    if let Some(fetch) = app.services.check_update.clone() {
        app.update.started = true;
        app.session.update_prefs.last_check_unix = now;
        app.update.rx = spawn(fetch, ctx.clone());
    }
}

fn collect_fetch(app: &mut WordApp) {
    let Some(rx) = &app.update.rx else { return };
    match rx.try_recv() {
        Ok(Ok(json)) => {
            app.update.rx = None;
            let current = parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0));
            if let Ok(Some(rel)) = parse_latest_release(&json, current) {
                if wordcraft_engine::update::skipped(&rel, &app.session.update_prefs.skipped_version) {
                    return;
                }
                let exe = app.session.update_exe.as_deref().unwrap_or("");
                let kind = wordcraft_engine::update::detect_install_kind(exe, &wordcraft_engine::update::SystemEnv);
                let off = offer(rel, &app.session.update_prefs.skipped_version, kind);
                app.update.notice = Some(Notice {
                    version: off.version,
                    notes: off.notes,
                    page_url: off.page_url,
                    one_click: off.one_click,
                    external_updater: off.external_updater,
                });
            }
        }
        Ok(Err(e)) => {
            log::info!("update check failed: {e}");
            app.update.rx = None;
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Err(std::sync::mpsc::TryRecvError::Disconnected) => app.update.rx = None,
    }
}

/// Manual check or command-driven notice (`file.checkForUpdates` ui_request).
pub fn apply_command_result(app: &mut WordApp, value: &Value) {
    if value.get("status") == Some(&Value::String("available".into()))
        && let Some(rel) = value.get("release")
    {
        app.update.notice = Some(Notice {
            version: rel.get("version").and_then(Value::as_str).unwrap_or("").into(),
            notes: rel.get("notes").and_then(Value::as_str).unwrap_or("").into(),
            page_url: rel.get("pageUrl").and_then(Value::as_str).unwrap_or(wordcraft_engine::update::RELEASES_PAGE).into(),
            one_click: rel.get("oneClick").and_then(Value::as_bool) == Some(true),
            external_updater: rel.get("externalUpdater").and_then(Value::as_bool) == Some(true),
        });
    }
}

pub fn bar(app: &mut WordApp, ui: &mut Ui) {
    let Some(notice) = app.update.notice.clone() else { return };
    let t = Tokens::get(ui.ctx());
    let mut close = false;
    egui::Panel::top("update_notice")
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(12, 6)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                    let text = crate::i18n::fmt(tl!("WordCraft {version} is available."), &[("version", &notice.version)]);
                    ui.label(egui::RichText::new(text).font(TypeRung::Control.medium()).color(t.text));
                    if notice.one_click && !notice.external_updater {
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
                            } else if app.run("file.checkForUpdates", json!({"force": true, "apply": true})).is_ok() {
                                close = true;
                            }
                        }
                    } else if ui.button(tl!("Download")).clicked() {
                        app.canvas.open_url = Some(notice.page_url.clone());
                        close = true;
                    }
                    if ui.button(tl!("Skip this version")).clicked() {
                        app.session.update_prefs.skipped_version = notice.version.clone();
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
            ui.label(tl!("Once a day, WordCraft can ask GitHub whether a newer release is out. Nothing else is sent."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("Yes")).clicked() {
                    app.session.update_prefs.check_on_startup = true;
                    app.session.update_prefs.asked_startup = true;
                    *open = false;
                }
                if ui.button(tl!("No")).clicked() {
                    app.session.update_prefs.check_on_startup = false;
                    app.session.update_prefs.asked_startup = true;
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

#[cfg(not(target_arch = "wasm32"))]
fn spawn(fetch: Fetch, ctx: egui::Context) -> Option<Receiver<Result<Vec<u8>, String>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    match std::thread::Builder::new().name("update-check".into()).spawn(move || {
        let _ = tx.send(fetch());
        ctx.request_repaint();
    }) {
        Ok(_) => Some(rx),
        Err(e) => {
            log::info!("update check not started: {e}");
            None
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn(_fetch: Fetch, _ctx: egui::Context) -> Option<Receiver<Result<Vec<u8>, String>>> {
    None
}

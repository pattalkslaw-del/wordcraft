//! Desktop update notice (#15, #201). Checks go through `file.checkForUpdates`; this module
//! paints the notice bar.

use egui::{Stroke, Ui, vec2};
use serde_json::{Value, json};

use crate::WordApp;
use crate::theme::{Tokens, TypeRung};

pub use wordcraft_engine::update::{INTERVAL_SECS, MAX_JSON, due};

#[derive(Default)]
pub struct UpdateCheck {
    started: bool,
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
        external_updater: v.get("externalUpdater").and_then(Value::as_bool) == Some(true),
    })
}

pub fn poll(app: &mut WordApp, _ctx: &egui::Context) {
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
    if let Err(e) = app.run("file.checkForUpdates", json!({"startup": true})) {
        log::info!("startup update check failed: {e}");
    }
}

/// Manual check or command-driven notice (`file.checkForUpdates` ui_request).
pub fn apply_command_result(app: &mut WordApp, value: &Value) {
    if let Some(n) = notice_from_payload(value) {
        app.update.notice = Some(n);
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
                            } else {
                                match app.run("file.checkForUpdates", json!({"force": true, "apply": true})) {
                                    Ok(v) => {
                                        if v.get("status") == Some(&Value::String("error".into())) {
                                            // The command reports a catalogued English key plus the
                                            // technical cause; translate the key, log the detail.
                                            let key = v.get("error").and_then(Value::as_str).unwrap_or("");
                                            let text = if key.is_empty() {
                                                tl!("The update could not be installed.").to_string()
                                            } else {
                                                tl!(key).to_string()
                                            };
                                            if let Some(detail) = v.get("errorDetail").and_then(Value::as_str) {
                                                log::warn!("update failed: {detail}");
                                            }
                                            app.status(text);
                                        } else if app.quit_requested {
                                            close = true;
                                        }
                                    }
                                    Err(e) => {
                                        log::warn!("update failed: {e}");
                                        app.status(tl!(wordcraft_engine::update::MSG_START_FAILED));
                                    }
                                }
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
}

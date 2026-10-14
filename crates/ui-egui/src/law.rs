//! File › Options › LawCraft: court profiles (list, edit, import, export, approve).

use egui::{RichText, Ui, vec2};
use serde_json::{Value, json};
use wordcraft_law::{MODULE, install, set_profiles_dir, settings_dir};

use crate::WordApp;
use crate::file_dialogs::{AfterPick, FileDialogRequest};
use crate::theme::{Tokens, semibold};

/// UI state for the LawCraft options page.
#[derive(Clone, Debug, Default)]
pub struct LawUi {
    /// Profile id selected in the list.
    pub selected: Option<String>,
    /// Document type being edited for the selected profile.
    pub doc_type: String,
    /// Waiting for the person to confirm approval.
    pub confirm_approve: bool,
    /// Last validation message from `law.profiles.validate` (every problem, one block).
    pub validation: Option<String>,
    /// Profile id a save dialog is exporting (`None` during import).
    pub export_id: Option<String>,
}

/// Install LawCraft commands when the module is on (safe to call every frame).
pub fn sync_module(app: &mut WordApp) {
    if app.session.module_enabled(MODULE) {
        install(&mut app.session);
    }
}

/// Whether LawCraft is on ([`MODULE`] in editing preferences).
pub fn enabled(app: &WordApp) -> bool {
    app.session.module_enabled(MODULE)
}

/// Turn LawCraft on or off and install its commands when enabling.
pub fn set_enabled(app: &mut WordApp, on: bool) {
    if on {
        app.session.prefs.modules.insert(MODULE.into(), true);
        set_profiles_dir(settings_dir());
        sync_module(app);
        app.ui.options_pane = "law".into();
    } else {
        app.session.prefs.modules.remove(MODULE);
        if app.ui.options_pane == "law" {
            app.ui.options_pane = "addins".into();
        }
    }
}

/// File › Options › Add-ins: optional modules.
pub fn addins(app: &mut WordApp, ui: &mut Ui) {
    crate::options::section(ui, "Add-ins");
    ui.label(tl!("Optional modules add ribbon commands and settings pages. They stay off until you turn them on."));
    ui.add_space(6.0);
    let on = enabled(app);
    let mut checked = on;
    if ui.checkbox(&mut checked, tl!("LawCraft (court profiles and legal formatting)")).changed() {
        set_enabled(app, checked);
    }
    ui.label(RichText::new(tl!("Legal court profiles, formatting rules and filing checks. See docs/law.md.")).small().weak());
}

/// The LawCraft settings page (only when the module is on).
pub fn page(app: &mut WordApp, ui: &mut Ui) {
    sync_module(app);
    if !enabled(app) {
        ui.label(tl!("Turn on LawCraft under Add-ins to manage court profiles."));
        return;
    }
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("LawCraft")).font(semibold(15.0)));
    ui.label(
        RichText::new(tl!(
            "Court profiles list the sections, styles and checks for each filing type. Edits save the profile as unverified until you approve it."
        ))
        .small()
        .weak(),
    );
    ui.add_space(8.0);

    let list = match app.run("law.profiles.list", json!({})) {
        Ok(v) => v.as_array().cloned().unwrap_or_default(),
        Err(e) => {
            ui.colored_label(ui.visuals().error_fg_color, e);
            return;
        }
    };

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(240.0);
            ui.label(RichText::new(tl!("Court profiles")).font(semibold(14.0)));
            egui::ScrollArea::vertical().id_salt("law_profiles").max_height(360.0).show(ui, |ui| {
                for row in &list {
                    let id = row["id"].as_str().unwrap_or("");
                    let name = row["name"].as_str().unwrap_or(id);
                    let jurisdiction = row["jurisdiction"].as_str().unwrap_or("");
                    let verified = row["verified"].as_bool() == Some(true);
                    let active = app.law.selected.as_deref() == Some(id);
                    let badge = if verified { tl!("Verified") } else { tl!("Unverified") };
                    let mut clicked = false;
                    ui.horizontal(|ui| {
                        clicked = ui.selectable_label(active, name).clicked();
                        ui.label(RichText::new(jurisdiction).small().weak());
                        ui.label(RichText::new(badge).small().color(if verified {
                            egui::Color32::from_rgb(0x10, 0x7c, 0x10)
                        } else {
                            ui.visuals().warn_fg_color
                        }));
                    });
                    if clicked {
                        app.law.selected = Some(id.to_string());
                        app.law.validation = None;
                        app.law.confirm_approve = false;
                        if let Ok(p) = app.run("law.profiles.get", json!({"id": id})) {
                            let docs = p["docs"].as_object();
                            let first = docs.and_then(|d| d.keys().next()).map(String::from).unwrap_or_default();
                            if app.law.doc_type.is_empty() || docs.is_none_or(|d| !d.contains_key(&app.law.doc_type)) {
                                app.law.doc_type = first;
                            }
                        }
                    }
                    ui.label(RichText::new(id).small().weak());
                    ui.add_space(4.0);
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("Import…")).clicked() {
                    let _ = app.ask_file(FileDialogRequest::Open { purpose: "lawProfile".into() }, AfterPick::LawImport);
                }
                let id = app.law.selected.clone();
                ui.add_enabled_ui(id.is_some(), |ui| {
                    if ui.button(tl!("Export…")).clicked() {
                        let name = format!("{}.json", id.as_deref().unwrap_or("profile"));
                        app.law.export_id = id;
                        let _ = app.ask_file(FileDialogRequest::Save { name }, AfterPick::LawExport);
                    }
                });
            });
        });
        ui.add_space(12.0);
        let (line, _) = ui.allocate_exact_size(vec2(1.0, 400.0), egui::Sense::hover());
        ui.painter().rect_filled(line, 0.0, t.border);
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_min_width(420.0);
            if let Some(id) = app.law.selected.clone() {
                profile_editor(app, ui, &id, &list);
            } else {
                ui.label(tl!("Select a court profile to view or edit it."));
            }
        });
    });
}

fn profile_editor(app: &mut WordApp, ui: &mut Ui, id: &str, list: &[Value]) {
    let summary = list.iter().find(|r| r["id"].as_str() == Some(id));
    let verified = summary.and_then(|r| r["verified"].as_bool()).unwrap_or(false);
    let mut profile = match app.run("law.profiles.get", json!({"id": id})) {
        Ok(p) => p,
        Err(e) => {
            ui.colored_label(ui.visuals().error_fg_color, e);
            return;
        }
    };

    ui.horizontal(|ui| {
        ui.label(RichText::new(profile["name"].as_str().unwrap_or(id)).font(semibold(16.0)));
        let badge = if verified { tl!("Verified") } else { tl!("Unverified") };
        let color = if verified { egui::Color32::from_rgb(0x10, 0x7c, 0x10) } else { ui.visuals().warn_fg_color };
        ui.label(RichText::new(badge).color(color));
    });

    if let Some(msg) = app.law.validation.as_ref().filter(|s| !s.is_empty()) {
        ui.add_space(4.0);
        ui.colored_label(ui.visuals().error_fg_color, msg);
    }

    ui.add_space(6.0);
    let docs: Vec<String> = profile["docs"].as_object().map(|d| d.keys().cloned().collect()).unwrap_or_default();
    if docs.is_empty() {
        ui.label(tl!("This profile has no document types."));
        return;
    }
    ui.horizontal(|ui| {
        ui.label(tl!("Document type:"));
        egui::ComboBox::from_id_salt("law_doc_type").selected_text(app.law.doc_type.as_str()).width(220.0).show_ui(ui, |ui| {
            for d in &docs {
                if ui.selectable_label(app.law.doc_type == *d, d.as_str()).clicked() {
                    app.law.doc_type = d.clone();
                }
            }
        });
    });

    let doc_key = app.law.doc_type.clone();
    let doc = profile["docs"].get(&doc_key).cloned().unwrap_or(Value::Null);

    crate::options::section(ui, "Sections");
    sections_editor(app, ui, id, &doc_key, &doc, &mut profile);

    crate::options::section(ui, "Styles");
    styles_editor(app, ui, id, &doc_key, &doc, &mut profile);

    crate::options::section(ui, "Checks");
    checks_editor(app, ui, id, &doc_key, &doc, &mut profile);

    ui.add_space(8.0);
    if !verified {
        approve_ui(app, ui, id);
    }
}

fn approve_ui(app: &mut WordApp, ui: &mut Ui, id: &str) {
    if app.law.confirm_approve {
        ui.label(tl!("Approve this court profile? You are confirming that its rules match the court's requirements."));
        ui.horizontal(|ui| {
            if ui.button(tl!("Approve")).clicked() {
                match app.run("law.profiles.approve", json!({"id": id})) {
                    Ok(_) => {
                        app.law.confirm_approve = false;
                        app.law.validation = None;
                        app.status(tl!("Court profile approved."));
                    }
                    Err(e) => app.status(e),
                }
            }
            if ui.button(tl!("Cancel")).clicked() {
                app.law.confirm_approve = false;
            }
        });
    } else if ui.button(tl!("Approve profile…")).clicked() {
        app.law.confirm_approve = true;
    }
}

fn sections_editor(app: &mut WordApp, ui: &mut Ui, profile_id: &str, doc_key: &str, doc: &Value, profile: &mut Value) {
    let sections = doc.get("sections").and_then(Value::as_array).cloned().unwrap_or_default();
    if sections.is_empty() {
        ui.label(tl!("No sections defined for this document type."));
        return;
    }
    let required = required_section_ids(doc);
    for (i, sec) in sections.iter().enumerate() {
        let sid = sec["id"].as_str().unwrap_or("");
        let mut name = sec["name"].as_str().unwrap_or("").to_string();
        let is_req = required.iter().any(|r| r == sid);
        let mut req = is_req;
        ui.horizontal(|ui| {
            ui.label(format!("{}.", i + 1));
            let resp = ui.text_edit_singleline(&mut name);
            ui.checkbox(&mut req, tl!("Required"));
            if ui.add_enabled(i > 0, egui::Button::new("↑")).clicked() {
                reorder_sections(app, profile_id, doc_key, profile, i, i - 1);
            }
            if ui.add_enabled(i + 1 < sections.len(), egui::Button::new("↓")).clicked() {
                reorder_sections(app, profile_id, doc_key, profile, i, i + 1);
            }
            let name_save = resp.lost_focus() && resp.changed();
            if name_save || req != is_req {
                let mut secs = profile["docs"][doc_key]["sections"].as_array().cloned().unwrap_or(sections.clone());
                if let Some(s) = secs.get_mut(i).and_then(Value::as_object_mut) {
                    s.insert("name".into(), json!(name.trim()));
                }
                if req != is_req {
                    set_section_required_in_profile(profile, doc_key, sid, req);
                    if let Some(ch) = profile["docs"][doc_key]["checks"].as_array() {
                        save_doc(app, profile_id, doc_key, profile, json!({"sections": secs, "checks": ch}));
                        return;
                    }
                }
                save_doc(app, profile_id, doc_key, profile, json!({"sections": secs}));
            }
        });
    }
}

fn required_section_ids(doc: &Value) -> Vec<String> {
    doc.get("checks")
        .and_then(Value::as_array)
        .map(|checks| {
            checks
                .iter()
                .filter(|c| c["kind"].as_str() == Some("requiredSections"))
                .flat_map(|c| c["ids"].as_array().into_iter().flat_map(|a| a.iter().filter_map(|v| v.as_str().map(String::from))))
                .collect()
        })
        .unwrap_or_default()
}

fn set_section_required_in_profile(profile: &mut Value, doc_key: &str, section_id: &str, required: bool) {
    let doc = profile["docs"].get(doc_key).cloned().unwrap_or(Value::Null);
    let checks = doc.get("checks").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut checks = checks;
    let mut found = false;
    for c in &mut checks {
        if c.get("kind").and_then(Value::as_str) == Some("requiredSections") {
            found = true;
            let ids = c.get("ids").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut ids: Vec<Value> = ids;
            let present = ids.iter().any(|v| v.as_str() == Some(section_id));
            if required && !present {
                ids.push(json!(section_id));
            } else if !required && present {
                ids.retain(|v| v.as_str() != Some(section_id));
            }
            if let Some(o) = c.as_object_mut() {
                o.insert("ids".into(), json!(ids));
            }
        }
    }
    if required && !found {
        checks.push(json!({
            "kind": "requiredSections",
            "ids": [section_id],
            "source": "Edited in LawCraft settings"
        }));
    }
    if let Some(docs) = profile.get_mut("docs").and_then(Value::as_object_mut)
        && let Some(d) = docs.get_mut(doc_key).and_then(Value::as_object_mut)
    {
        d.insert("checks".into(), json!(checks));
    }
}

fn reorder_sections(app: &mut WordApp, profile_id: &str, doc_key: &str, profile: &mut Value, from: usize, to: usize) {
    let doc = profile["docs"].get(doc_key).cloned().unwrap_or(Value::Null);
    let mut sections = doc.get("sections").and_then(Value::as_array).cloned().unwrap_or_default();
    if from >= sections.len() || to >= sections.len() {
        return;
    }
    let item = sections.remove(from);
    sections.insert(to, item);
    save_doc(app, profile_id, doc_key, profile, json!({"sections": sections}));
}

fn styles_editor(app: &mut WordApp, ui: &mut Ui, profile_id: &str, doc_key: &str, doc: &Value, profile: &mut Value) {
    let styles = doc.get("styles").and_then(Value::as_array).cloned().unwrap_or_default();
    if styles.is_empty() {
        ui.label(tl!("No styles defined for this document type."));
        return;
    }
    egui::ScrollArea::vertical().id_salt("law_styles").max_height(200.0).show(ui, |ui| {
        for (i, st) in styles.iter().enumerate() {
            let name = st["name"].as_str().unwrap_or("");
            ui.collapsing(name, |ui| {
                let mut font = st.pointer("/chr/font").and_then(Value::as_str).unwrap_or("").to_string();
                let mut size = st.pointer("/chr/size").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                ui.horizontal(|ui| {
                    ui.label(tl!("Font:"));
                    let font_changed = ui.text_edit_singleline(&mut font).changed();
                    ui.label(tl!("Size (pt):"));
                    let size_changed = ui.add(egui::DragValue::new(&mut size).speed(0.5).range(1.0..=4096.0)).changed();
                    if font_changed || size_changed {
                        let mut styles = styles.clone();
                        if let Some(s) = styles.get_mut(i).and_then(Value::as_object_mut) {
                            let chr = s.entry("chr").or_insert(json!({}));
                            if let Some(c) = chr.as_object_mut() {
                                if !font.trim().is_empty() {
                                    c.insert("font".into(), json!(font.trim()));
                                }
                                if size > 0.0 {
                                    c.insert("size".into(), json!(size));
                                }
                            }
                        }
                        save_doc(app, profile_id, doc_key, profile, json!({"styles": styles}));
                    }
                });
                let mut spacing = st.pointer("/para/lineSpacing/value").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                ui.horizontal(|ui| {
                    ui.label(tl!("Line spacing (multiple):"));
                    if ui.add(egui::DragValue::new(&mut spacing).speed(0.05).range(0.5..=4.0)).changed() {
                        let mut styles = styles.clone();
                        if let Some(s) = styles.get_mut(i).and_then(Value::as_object_mut) {
                            let para = s.entry("para").or_insert(json!({}));
                            if let Some(p) = para.as_object_mut() {
                                p.insert("lineSpacing".into(), json!({"rule": "multiple", "value": spacing}));
                            }
                        }
                        save_doc(app, profile_id, doc_key, profile, json!({"styles": styles}));
                    }
                });
            });
        }
    });
    ui.label(RichText::new(tl!("Full style editing uses the document's Modify Style dialog when you apply a profile to a document.")).small().weak());
}

fn checks_editor(app: &mut WordApp, ui: &mut Ui, profile_id: &str, doc_key: &str, doc: &Value, profile: &mut Value) {
    let checks = doc.get("checks").and_then(Value::as_array).cloned().unwrap_or_default();
    if checks.is_empty() {
        ui.label(tl!("No checks defined for this document type."));
        return;
    }
    for (i, ch) in checks.iter().enumerate() {
        let kind = ch["kind"].as_str().unwrap_or("");
        ui.horizontal(|ui| {
            ui.label(RichText::new(kind).font(semibold(13.0)));
            if kind == "wordLimit" {
                let mut limit = ch["limit"].as_u64().unwrap_or(0) as u32;
                ui.label(tl!("Word limit:"));
                if ui.add(egui::DragValue::new(&mut limit).speed(100.0).range(1..=1_000_000)).changed() {
                    let mut checks = checks.clone();
                    if let Some(c) = checks.get_mut(i).and_then(Value::as_object_mut) {
                        c.insert("limit".into(), json!(limit));
                    }
                    save_doc(app, profile_id, doc_key, profile, json!({"checks": checks}));
                }
            } else if kind == "minFontSize" {
                let mut pt = ch["pt"].as_f64().unwrap_or(0.0) as f32;
                ui.label(tl!("Minimum (pt):"));
                if ui.add(egui::DragValue::new(&mut pt).speed(0.5).range(1.0..=4096.0)).changed() {
                    let mut checks = checks.clone();
                    if let Some(c) = checks.get_mut(i).and_then(Value::as_object_mut) {
                        c.insert("pt".into(), json!(pt));
                    }
                    save_doc(app, profile_id, doc_key, profile, json!({"checks": checks}));
                }
            }
        });
        if let Some(src) = ch["source"].as_str().filter(|s| !s.is_empty()) {
            ui.label(RichText::new(src).small().weak());
        }
    }
}

fn save_doc(app: &mut WordApp, profile_id: &str, doc_key: &str, profile: &mut Value, patch: Value) {
    let doc = profile["docs"].get(doc_key).cloned().unwrap_or(Value::Null);
    let mut merged = doc.as_object().cloned().unwrap_or_default();
    if let Some(p) = patch.as_object() {
        for (k, v) in p {
            merged.insert(k.clone(), v.clone());
        }
    }
    if let Some(docs) = profile.get_mut("docs").and_then(Value::as_object_mut) {
        docs.insert(doc_key.to_string(), Value::Object(merged));
    }
    let docs = profile["docs"].clone();
    match app.run("law.profiles.update", json!({"id": profile_id, "docs": docs})) {
        Ok(updated) => {
            *profile = updated;
            validate_profile(app, profile_id);
        }
        Err(e) => {
            app.law.validation = Some(e.clone());
            app.status(e);
        }
    }
}

fn validate_profile(app: &mut WordApp, profile_id: &str) {
    match app.run("law.profiles.get", json!({"id": profile_id})) {
        Ok(p) => match app.run("law.profiles.validate", p) {
            Ok(_) => app.law.validation = None,
            Err(e) => app.law.validation = Some(e),
        },
        Err(e) => app.law.validation = Some(e),
    }
}

/// Finish a file pick for LawCraft import or export.
pub(crate) fn finish_pick(app: &mut WordApp, after: AfterPick, picked: Option<String>) -> Result<Value, String> {
    match after {
        AfterPick::LawImport => {
            let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
            let r = app.run("law.profiles.import", json!({"path": path}));
            if let Ok(v) = &r {
                if let Some(id) = v.get("id").and_then(Value::as_str) {
                    app.law.selected = Some(id.to_string());
                    app.law.validation = None;
                }
                app.status(crate::i18n::fmt(tl!("Imported court profile {path}"), &[("path", &path)]));
            }
            r
        }
        AfterPick::LawExport => {
            let Some(path) = picked else {
                app.law.export_id = None;
                return Ok(json!({"cancelled": true}));
            };
            let id = app.law.export_id.clone().ok_or_else(|| "no profile selected for export".to_string())?;
            app.law.export_id = None;
            let r = app.run("law.profiles.export", json!({"id": id, "path": path}));
            if r.is_ok() {
                app.status(crate::i18n::fmt(tl!("Exported court profile {path}"), &[("path", &path)]));
            }
            r
        }
        _ => Err("not a LawCraft file pick".into()),
    }
}

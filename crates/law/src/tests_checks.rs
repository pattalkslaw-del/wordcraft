//! Court profile checks: compliant and violating documents per bundled profile.

use serde_json::{Value, json};
use wordcraft_doc::StoryRef;
use wordcraft_engine::Session;

use crate::cmd::store;
use crate::profile::Profile;
use crate::specs;

use super::tests::{FEDERAL, MISSOURI, apply_session, err, law, run};

fn findings(s: &mut Session, id: &str, doc_type: &str, for_export: bool) -> Vec<Value> {
    let r = run(s, "law.checks.run", json!({"id": id, "doc": doc_type, "forExport": for_export, "store": false}));
    r["findings"].as_array().cloned().unwrap_or_default()
}

fn has_check(findings: &[Value], check_id: &str) -> bool {
    findings.iter().any(|f| f["checkId"] == json!(check_id))
}

/// Write every section heading from the profile, with body text under each.
fn write_skeleton(s: &mut Session, profile: &Profile, doc_type: &str) {
    let spec = profile.docs.get(doc_type).expect("document type");
    for sec in &spec.sections {
        run(s, "text.insert", json!({"text": sec.name}));
        if let Some(st) = &sec.style {
            run(s, "para.style", json!({"style": st}));
        }
        run(s, "text.insert", json!({"text": "\r"}));
        run(s, "styles.apply", json!({"style": "Body"}));
        run(s, "text.insert", json!({"text": "Body paragraph for this section."}));
        run(s, "text.insert", json!({"text": "\r"}));
    }
}

#[test]
fn missouri_initial_brief_skeleton_passes_checks() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    let f = findings(&mut s, MISSOURI, "brief", false);
    assert!(f.is_empty(), "expected no findings: {f:?}");
}

#[test]
fn federal_principal_brief_skeleton_passes_checks() {
    let profile = store().get(FEDERAL).expect("profile").clone();
    let mut s = apply_session(FEDERAL, "brief");
    write_skeleton(&mut s, &profile, "brief");
    let f = findings(&mut s, FEDERAL, "brief", false);
    assert!(f.is_empty(), "expected no findings: {f:?}");
}

#[test]
fn each_missouri_check_fires_on_a_violation() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    // Word limit (use a tiny limit on a test profile)
    let mut s = law();
    let mut p = super::tests::profile_json("test-word-limit");
    p["docs"]["brief"]["checks"] = json!([{"kind": "wordLimit", "limit": 5, "source": "Test Rule"}]);
    run(&mut s, "law.profiles.create", p);
    run(&mut s, "law.profiles.apply", json!({"id": "test-word-limit", "doc": "brief"}));
    run(&mut s, "text.insert", json!({"text": "one two three four five six seven"}));
    assert!(has_check(&findings(&mut s, "test-word-limit", "brief", false), "wordLimit"));
    // Required section missing
    let mut s = apply_session(MISSOURI, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Cover\n"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "requiredSections"));
    // Hyperlink
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "link"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "noHyperlinks"));
    // Small type
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "format.size", json!({"size": 10}));
    run(&mut s, "text.insert", json!({"text": "tiny"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "minFontSize"));
    // Tight margins
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    assert!(s.run("layout.margins", &json!({"top": 36})).is_ok());
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "minMargins"));
}

#[test]
fn export_runs_checks_and_reports_through_proofing() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    let dir = super::tests::temp_dir("export-checks");
    let path = dir.join("brief.pdf");
    let r = run(&mut s, "file.exportPdf", json!({"path": path.to_string_lossy()}));
    let export_findings = r["findings"].as_array().expect("export findings");
    assert!(export_findings.iter().any(|f| f["checkId"] == "noHyperlinks"));
    let issues = run(&mut s, "review.issues", json!({}));
    let issues = issues.as_array().expect("issues");
    assert!(issues.iter().any(|f| f["kind"] == "lint" && f["checkId"] == "noHyperlinks"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn law_checks_run_is_pure_and_lists_findings() {
    let mut s = law();
    let e = err(&mut s, "law.checks.run", json!({})).to_string();
    assert!(e.contains("profile"), "{e}");
    assert!(specs().iter().any(|c| c.id == "law.checks.run"));
}

#[test]
fn export_checks_survive_save_and_reopen() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    let bytes = wordcraft_engine::io::save_bytes("brief.docx", &s.doc).expect("save");
    let doc = wordcraft_engine::io::open_bytes("brief.docx", &bytes).expect("open");
    let mut s = law();
    s.set_document(doc);
    let dir = super::tests::temp_dir("reopen-export");
    let path = dir.join("brief.pdf");
    let r = run(&mut s, "file.exportPdf", json!({"path": path.to_string_lossy()}));
    assert!(r["findings"].as_array().expect("findings").iter().any(|f| f["checkId"] == "noHyperlinks"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn later_section_tight_margins_are_reported() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "layout.break", json!({"kind": "nextPage"}));
    run(&mut s, "text.insert", json!({"text": "Second section body\r"}));
    assert!(s.run("layout.margins", &json!({"top": 36})).is_ok());
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "minMargins"));
}

#[test]
fn exact_line_spacing_below_minimum_fails() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "styles.apply", json!({"style": "Body"}));
    run(&mut s, "para.lineSpacing", json!({"exactly": 12}));
    run(&mut s, "text.insert", json!({"text": "Tight body line.\r"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "minLineSpacing"));
}

#[test]
fn footnote_hyperlink_is_reported() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    let note = StoryRef::Part(run(&mut s, "references.footnote", json!({"text": "See also"}))["id"].as_u64().expect("id") as u32);
    run(&mut s, "caret.set", json!({"pos": {"story": note, "path": [0], "off": 0}}));
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "link"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "noHyperlinks"));
}

#[test]
fn text_before_first_heading_counts_toward_word_limit() {
    let mut s = law();
    let mut p = super::tests::profile_json("test-pre-heading");
    p["docs"]["brief"]["checks"] = json!([{"kind": "wordLimit", "limit": 3, "source": "Test"}]);
    run(&mut s, "law.profiles.create", p);
    run(&mut s, "law.profiles.apply", json!({"id": "test-pre-heading", "doc": "brief"}));
    run(&mut s, "text.insert", json!({"text": "one two three four five six\r"}));
    run(&mut s, "text.insert", json!({"text": "Cover\r"}));
    assert!(has_check(&findings(&mut s, "test-pre-heading", "brief", false), "wordLimit"));
}

#[test]
fn wrong_typeface_fails_missouri_check() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "format.font", json!({"name": "Arial"}));
    run(&mut s, "text.insert", json!({"text": "wrong face"}));
    assert!(has_check(&findings(&mut s, MISSOURI, "brief", false), "minFontSize"));
}

#[test]
fn federal_small_type_fails_check() {
    let profile = store().get(FEDERAL).expect("profile").clone();
    let mut s = apply_session(FEDERAL, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "format.size", json!({"size": 10}));
    run(&mut s, "text.insert", json!({"text": "small"}));
    assert!(has_check(&findings(&mut s, FEDERAL, "brief", false), "minFontSize"));
}

#[test]
fn searchable_pdf_passes_on_real_export() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    let f = findings(&mut s, MISSOURI, "brief", true);
    assert!(!has_check(&f, "textSearchablePdf"), "{f:?}");
}

#[test]
fn font_only_pdf_bytes_are_not_searchable() {
    let bytes = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/MediaBox[0 0 200 200]/Resources<</Font<</F1 4 0 R>>>>>>endobj\n4 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
    assert!(!wordcraft_pdf::has_extractable_text(bytes));
}

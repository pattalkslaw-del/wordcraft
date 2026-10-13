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
    // Not every document type has a `Body` style (the appendix does not); fall back to the first
    // style so the body still carries the profile's formatting.
    let body_style = if spec.styles.iter().any(|st| st.name.eq_ignore_ascii_case("Body")) {
        "Body"
    } else {
        spec.styles.first().map(|st| st.name.as_str()).unwrap_or("Normal")
    };
    for sec in &spec.sections {
        run(s, "text.insert", json!({"text": sec.name}));
        if let Some(st) = &sec.style {
            run(s, "para.style", json!({"style": st}));
        }
        run(s, "text.insert", json!({"text": "\r"}));
        run(s, "styles.apply", json!({"style": body_style}));
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

/// Every document type of every bundled profile: a compliant skeleton reports nothing.
#[test]
fn every_bundled_document_type_compliant_skeleton_has_no_findings() {
    for id in [MISSOURI, FEDERAL] {
        let profile = store().get(id).expect("profile").clone();
        let doc_types: Vec<String> = profile.doc_types().iter().map(|s| s.to_string()).collect();
        assert!(!doc_types.is_empty(), "{id} has no document types");
        for doc_type in doc_types {
            let mut s = apply_session(id, &doc_type);
            write_skeleton(&mut s, &profile, &doc_type);
            let f = findings(&mut s, id, &doc_type, false);
            assert!(f.is_empty(), "{id}/{doc_type}: expected no findings: {f:?}");
        }
    }
}

/// The bundled respondent and reply briefs carry the formatting checks too, not just a typeface.
#[test]
fn missouri_respondent_and_reply_checks_are_populated() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    for doc_type in ["respondentBrief", "reply"] {
        let spec = profile.docs.get(doc_type).expect("document type");
        let kinds: Vec<&str> = spec.checks.iter().map(|c| c.kind.as_str()).collect();
        for want in ["wordLimit", "minFontSize", "minMargins", "minLineSpacing", "requiredSections", "noHyperlinks", "textSearchablePdf"] {
            assert!(kinds.contains(&want), "{doc_type} is missing `{want}`: {kinds:?}");
        }
        let word_limit = spec.checks.iter().find(|c| c.kind == "wordLimit").expect("a word limit");
        assert!(!word_limit.exclude_sections.is_empty(), "{doc_type}: the word limit needs counting exclusions");
        let sections = spec.checks.iter().find(|c| c.kind == "requiredSections").expect("required sections");
        assert_eq!(sections.ids.len(), spec.sections.len(), "{doc_type}: every section is required, in order");
    }
}

/// Respondent and reply briefs report the same violations as the initial brief.
#[test]
fn missouri_respondent_and_reply_violations_are_reported() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    for doc_type in ["respondentBrief", "reply"] {
        // Small type.
        let mut s = apply_session(MISSOURI, doc_type);
        write_skeleton(&mut s, &profile, doc_type);
        run(&mut s, "format.size", json!({"size": 10}));
        run(&mut s, "text.insert", json!({"text": "tiny"}));
        assert!(has_check(&findings(&mut s, MISSOURI, doc_type, false), "minFontSize"), "{doc_type}: minFontSize");
        // Tight margins.
        let mut s = apply_session(MISSOURI, doc_type);
        write_skeleton(&mut s, &profile, doc_type);
        assert!(s.run("layout.margins", &json!({"top": 36})).is_ok());
        assert!(has_check(&findings(&mut s, MISSOURI, doc_type, false), "minMargins"), "{doc_type}: minMargins");
        // A required section missing (or out of order).
        let mut s = apply_session(MISSOURI, doc_type);
        run(&mut s, "styles.apply", json!({"style": "Heading"}));
        run(&mut s, "text.insert", json!({"text": "Cover\n"}));
        assert!(has_check(&findings(&mut s, MISSOURI, doc_type, false), "requiredSections"), "{doc_type}: requiredSections");
    }
}

/// Federal violations beyond the type size: margins, spacing and section order.
#[test]
fn federal_checks_fire_beyond_font_size() {
    let profile = store().get(FEDERAL).expect("profile").clone();
    // Tight margins.
    let mut s = apply_session(FEDERAL, "brief");
    write_skeleton(&mut s, &profile, "brief");
    assert!(s.run("layout.margins", &json!({"top": 36})).is_ok());
    assert!(has_check(&findings(&mut s, FEDERAL, "brief", false), "minMargins"));
    // Single-spaced body below FRAP's double-spacing floor.
    let mut s = apply_session(FEDERAL, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "styles.apply", json!({"style": "Body"}));
    run(&mut s, "para.lineSpacing", json!({"value": 1}));
    run(&mut s, "text.insert", json!({"text": "Tight.\r"}));
    assert!(has_check(&findings(&mut s, FEDERAL, "brief", false), "minLineSpacing"));
    // A required section missing.
    let mut s = apply_session(FEDERAL, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Argument\n"}));
    assert!(has_check(&findings(&mut s, FEDERAL, "brief", false), "requiredSections"));
}

/// Required sections must be present *and in order*: two sections in the wrong order are reported.
#[test]
fn sections_out_of_order_are_reported() {
    let mut s = apply_session(MISSOURI, "brief");
    // Conclusion before Argument: both are present, but the order is wrong.
    for name in ["Conclusion", "Argument"] {
        run(&mut s, "styles.apply", json!({"style": "Heading"}));
        run(&mut s, "text.insert", json!({"text": format!("{name}\n")}));
    }
    let f = findings(&mut s, MISSOURI, "brief", false);
    assert!(has_check(&f, "requiredSections"), "{f:?}");
}

/// The word limit counts the profile's included sections and skips the excluded ones; the
/// boundary is exact.
#[test]
fn word_limit_counts_included_sections_only() {
    // The bundled Missouri brief, with a limit small enough to reach in a test: the same check,
    // the same `excludeSections`, only the number lowered.
    let mut profile = store().get(MISSOURI).expect("profile").clone();
    let brief = profile.docs.get_mut("brief").expect("brief");
    let wl = brief.checks.iter_mut().find(|c| c.kind == "wordLimit").expect("a word limit");
    wl.limit = Some(6);

    // Words under an excluded heading (Cover) do not count.
    let mut s = apply_session(MISSOURI, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Cover\r"}));
    run(&mut s, "styles.apply", json!({"style": "Body"}));
    run(&mut s, "text.insert", json!({"text": "one two three four five six seven eight\r"}));
    let f = crate::checks::evaluate(&s.doc, &profile, "brief", false, None).expect("evaluate");
    assert!(!f.iter().any(|x| x.check_id == "wordLimit"), "excluded words counted: {f:?}");

    // A counted heading (Argument) plus five words is exactly the limit: it passes...
    let mut s = apply_session(MISSOURI, "brief");
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.insert", json!({"text": "Argument\r"}));
    run(&mut s, "styles.apply", json!({"style": "Body"}));
    run(&mut s, "text.insert", json!({"text": "one two three four five\r"}));
    let f = crate::checks::evaluate(&s.doc, &profile, "brief", false, None).expect("evaluate");
    assert!(!f.iter().any(|x| x.check_id == "wordLimit"), "exactly at the limit must pass: {f:?}");
    // ...and one more word is over it.
    run(&mut s, "text.insert", json!({"text": "six\r"}));
    let f = crate::checks::evaluate(&s.doc, &profile, "brief", false, None).expect("evaluate");
    assert!(f.iter().any(|x| x.check_id == "wordLimit"), "one over the limit must fail: {f:?}");
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
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    // A missing profile id is an error (no applied profile and none given).
    let e = err(&mut s, "law.checks.run", json!({"id": "no-such-court", "doc": "brief"})).to_string();
    assert!(e.contains("no court profile"), "{e}");
    // Running the checks returns the findings and never changes the document.
    let before = serde_json::to_string(&s.doc).expect("serialize");
    let r = run(&mut s, "law.checks.run", json!({"id": MISSOURI, "doc": "brief", "store": false}));
    let after = serde_json::to_string(&s.doc).expect("serialize");
    assert_eq!(before, after, "law.checks.run must not edit the document");
    assert!(r["findings"].as_array().is_some_and(|f| has_check(f, "noHyperlinks")), "{r}");
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
    assert_eq!(wordcraft_pdf::text_searchability(bytes), wordcraft_pdf::TextSearchability::NotSearchable);
}

/// An unsearchable export produces the matching LawCraft finding, not a silent pass.
#[test]
fn unsearchable_pdf_produces_the_text_searchable_finding() {
    let font_only = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/MediaBox[0 0 200 200]/Resources<</Font<</F1 4 0 R>>>>>>endobj\n4 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
    let f = crate::checks::searchability_finding(font_only, "textSearchablePdf").expect("an unsearchable PDF must produce a finding");
    assert_eq!(f.check_id, "textSearchablePdf");
    assert_eq!(f.severity, "error");
    // A real export carries text, so the same helper says nothing.
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile_missouri(), "brief");
    let bytes = wordcraft_engine::io::save_bytes_opts("export.pdf", &s.doc, &Default::default()).expect("export a PDF");
    assert!(crate::checks::searchability_finding(&bytes, "textSearchablePdf").is_none());
}

fn profile_missouri() -> Profile {
    store().get(MISSOURI).expect("profile").clone()
}

/// A host that opens a file and *then* installs LawCraft (`Session::new(doc)` → `install`) must
/// still get the applied profile's export checks; it never calls `set_document`.
#[test]
fn install_restores_export_checks_for_an_already_open_document() {
    let profile = profile_missouri();
    let mut built = apply_session(MISSOURI, "brief");
    write_skeleton(&mut built, &profile, "brief");
    run(&mut built, "insert.link", json!({"url": "https://example.com", "text": "x"}));
    let bytes = wordcraft_engine::io::save_bytes("brief.docx", &built.doc).expect("save");
    let doc = wordcraft_engine::io::open_bytes("brief.docx", &bytes).expect("open");

    // A fresh host session over the reopened document, then LawCraft installed (no set_document).
    let mut s = Session::new(doc);
    s.prefs.modules.insert(crate::MODULE.into(), true);
    assert_eq!(crate::install(&mut s), specs().len());

    let dir = super::tests::temp_dir("install-export");
    let path = dir.join("brief.pdf");
    let r = run(&mut s, "file.exportPdf", json!({"path": path.to_string_lossy()}));
    let f = r["findings"].as_array().expect("findings");
    assert!(f.iter().any(|x| x["checkId"] == "noHyperlinks"), "install must restore export checks: {f:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

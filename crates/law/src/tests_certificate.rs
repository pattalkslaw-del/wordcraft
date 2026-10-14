//! Certificate of compliance: word count parity, refresh on export, and DOCX round trip.

use serde_json::json;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::Session;

use crate::certificate::FIELD_WORDCOUNT;
use crate::checks;
use crate::cmd::store;
use crate::specs;
use crate::tests_checks::write_skeleton;

use super::tests::{FEDERAL, MISSOURI, apply_session, law, roundtrip, run};

fn cert_wordcount_in_doc(doc: &Document) -> Option<u32> {
    for path in doc.para_paths(StoryRef::Body) {
        let para = doc.para(StoryRef::Body, &path)?;
        for obj in &para.objects {
            if let InlineObject::Field { instr, result, .. } = obj
                && instr == FIELD_WORDCOUNT
            {
                return result.parse().ok();
            }
        }
    }
    None
}

fn compliance(s: &mut Session, id: &str, doc_type: &str) -> serde_json::Value {
    run(s, "law.certificate.compliance", json!({"id": id, "doc": doc_type}))
}

#[test]
fn certificate_word_count_matches_the_word_limit_check() {
    for (id, doc_type) in [(MISSOURI, "brief"), (FEDERAL, "brief")] {
        let profile = store().get(id).expect("profile").clone();
        let mut s = apply_session(id, doc_type);
        write_skeleton(&mut s, &profile, doc_type);
        let r = compliance(&mut s, id, doc_type);
        let from_cert = r["wordCount"].as_u64().expect("wordCount") as u32;
        let from_check = checks::word_count(&s.doc, &profile, doc_type).expect("count");
        assert_eq!(from_cert, from_check, "{id}/{doc_type}");
        assert_eq!(cert_wordcount_in_doc(&s.doc), Some(from_check));
    }
}

#[test]
fn editing_then_save_refreshes_the_certificate_count() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    compliance(&mut s, MISSOURI, "brief");
    run(&mut s, "styles.apply", json!({"style": "Body"}));
    run(&mut s, "text.insert", json!({"text": "extra extra extra words added after the certificate.\r"}));
    let before = checks::word_count(&s.doc, &profile, "brief").expect("count");
    let dir = super::tests::temp_dir("cert-save");
    let path = dir.join("brief.docx");
    run(&mut s, "file.save", json!({"path": path.to_string_lossy()}));
    let after_field = cert_wordcount_in_doc(&s.doc).expect("field");
    assert_eq!(after_field, before, "export refresh must update the certificate field");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn certificate_field_survives_docx_round_trip() {
    let profile = store().get(FEDERAL).expect("profile").clone();
    let mut s = apply_session(FEDERAL, "brief");
    write_skeleton(&mut s, &profile, "brief");
    compliance(&mut s, FEDERAL, "brief");
    let doc = roundtrip(&s);
    assert!(doc.plain_text(StoryRef::Body).contains("Certificate of Compliance"));
    assert!(cert_wordcount_in_doc(&doc).is_some(), "LAWWORDCOUNT survives .docx");
}

#[test]
fn over_limit_still_inserts_and_returns_the_word_limit_finding() {
    let mut p = serde_json::to_value(store().get(MISSOURI).expect("profile")).expect("json");
    p["id"] = json!("test-cert-over");
    p["docs"]["brief"]["checks"] = json!([{
        "kind": "wordLimit",
        "limit": 5,
        "excludeSections": ["cover", "tableOfContents", "tableOfAuthorities", "certificateOfCompliance", "certificateOfService"],
        "source": "Test Rule"
    }]);
    p["docs"]["brief"]["certificateOfCompliance"] = json!({
        "template": "I certify that this brief contains {wordCount} words and complies with the limit of {wordLimit} words ({rule}).",
        "source": "Test Rule"
    });
    let mut s = law();
    run(&mut s, "law.profiles.create", p);
    run(&mut s, "law.profiles.apply", json!({"id": "test-cert-over", "doc": "brief"}));
    let profile = store().get("test-cert-over").expect("profile").clone();
    write_skeleton(&mut s, &profile, "brief");
    let r = compliance(&mut s, "test-cert-over", "brief");
    let findings = r["findings"].as_array().expect("findings");
    assert!(findings.iter().any(|f| f["checkId"] == "wordLimit"), "{findings:?}");
    assert!(s.doc.plain_text(StoryRef::Body).contains("I certify"));
}

#[test]
fn update_fields_refreshes_lawwordcount() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    compliance(&mut s, MISSOURI, "brief");
    run(&mut s, "text.insert", json!({"text": "more counted words here.\r"}));
    let want = checks::word_count(&s.doc, &profile, "brief").expect("count");
    run(&mut s, "references.updateFields", json!({}));
    assert_eq!(cert_wordcount_in_doc(&s.doc), Some(want));
}

#[test]
fn compliance_and_refresh_commands_are_installed() {
    let ids: Vec<&str> = specs().iter().map(|c| c.id).collect();
    assert!(ids.contains(&"law.certificate.compliance"));
    assert!(ids.contains(&"law.certificate.refresh"));
}

#[test]
fn install_registers_the_field_update_hook() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    compliance(&mut s, MISSOURI, "brief");
    run(&mut s, "text.insert", json!({"text": "hook test words.\r"}));
    let want = checks::word_count(&s.doc, &profile, "brief").expect("count");
    run(&mut s, "references.updateFields", json!({}));
    assert_eq!(cert_wordcount_in_doc(&s.doc), Some(want));
}

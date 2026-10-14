//! End-to-end: Missouri brief with every LawCraft command group in one session.

use serde_json::json;

use crate::cmd::store;

use super::tests::{MISSOURI, apply_session, run};
use super::tests_checks::write_skeleton;

#[test]
fn missouri_brief_runs_checks_proofing_citations_symbols_and_certificate() {
    let profile = store().get(MISSOURI).expect("profile").clone();
    let mut s = apply_session(MISSOURI, "brief");
    write_skeleton(&mut s, &profile, "brief");
    run(&mut s, "text.insert", json!({"text": "Thummel v. King, 570 S.W.2d 679 (Mo. banc 1978).\r"}));
    run(&mut s, "law.symbols.section", json!({}));
    run(&mut s, "text.insert", json!({"text": "1.010 RSMo.\r"}));

    let checks = run(&mut s, "law.checks.run", json!({"store": false}));
    assert_eq!(checks["profile"], json!(MISSOURI));
    assert_eq!(checks["doc"], json!("brief"));
    assert!(checks["findings"].as_array().is_some_and(|f| f.is_empty()), "{checks:?}");

    let confusions = run(&mut s, "law.proof.confusions", json!({"store": false}));
    assert!(confusions["findings"].as_array().is_some_and(|f| f.is_empty()), "{confusions:?}");

    let citations = run(&mut s, "law.citations.check", json!({"store": false}));
    assert_eq!(citations["pack"], json!("missouri"));
    assert!(citations["findings"].as_array().is_some_and(|f| f.is_empty()), "{citations:?}");

    let cert = run(&mut s, "law.certificate.compliance", json!({}));
    assert_eq!(cert["profile"], json!(MISSOURI));
    assert_eq!(cert["doc"], json!("brief"));
    assert!(cert["wordCount"].as_u64().is_some_and(|n| n > 0));
    assert!(cert["findings"].as_array().is_none_or(|f| f.is_empty()), "{cert:?}");

    let toa = run(&mut s, "law.toa.build", json!({"store": false}));
    assert_eq!(toa["pack"], json!("missouri"));
    assert!(toa["entries"].as_u64().is_some_and(|n| n > 0), "{toa:?}");
    let body = s.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(body.contains("570 S.W.2d 679"), "the table lists the cited case: {body}");

    let appendix = run(&mut s, "law.appendix.create", json!({}));
    assert_eq!(appendix["profile"], json!(MISSOURI));
    assert_eq!(appendix["doc"], json!("appendix"));
    assert_eq!(s.doc.custom_prop(crate::checks::PROP_DOC_TYPE), Some("appendix"));
    let findings = run(&mut s, "law.checks.run", json!({"doc": "appendix", "store": false}));
    assert!(findings["findings"].as_array().is_some_and(|f| f.is_empty()), "{findings:?}");
}

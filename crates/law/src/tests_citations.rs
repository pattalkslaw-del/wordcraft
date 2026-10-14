//! Citation form check: Missouri and federal examples from the LawCraft spec.

use serde_json::json;
use wordcraft_engine::Session;

use crate::cmd::store;
use crate::specs;

use super::tests::{FEDERAL, MISSOURI, apply_session, err, law, run};

fn check(s: &mut Session, pack: &str, store_findings: bool) -> Vec<serde_json::Value> {
    let r = run(s, "law.citations.check", json!({"pack": pack, "store": store_findings}));
    r["findings"].as_array().cloned().unwrap_or_default()
}

fn has_rule(findings: &[serde_json::Value], rule_id: &str) -> bool {
    findings.iter().any(|f| f["checkId"] == json!(rule_id))
}

fn insert(s: &mut Session, text: &str) {
    run(s, "text.insert", json!({"text": text}));
}

#[test]
fn missouri_good_citations_have_no_findings() {
    let mut s = law();
    for cite in [
        "Thummel v. King, 570 S.W.2d 679 (Mo. banc 1978)",
        "See also (Mo. App. E.D. 2020) and (Mo. App. W.D. 2019) and (Mo. App. S.D. 2021)",
        "Section 537.080, RSMo 2016",
        "§ 537.080 RSMo",
        "Rule 84.04(d)",
    ] {
        insert(&mut s, cite);
        insert(&mut s, "\r");
    }
    let f = check(&mut s, "missouri", false);
    assert!(f.is_empty(), "expected no findings: {f:?}");
}

#[test]
fn missouri_bad_citations_are_reported() {
    let cases: [(&str, &str); 6] = [
        ("Thummel v. King, 570 S. W. 2d 679 (Mo. banc 1978)", "moCaseReporter"),
        ("Thummel v. King, 570 SW2d 679 (Mo. banc 1978)", "moCaseReporterWrong"),
        ("Thummel v. King, 570 S.W.2d 679", "moCaseParenthetical"),
        ("Thummel v. King, 570 S.W.2d 679 (Mo. Banc 1978)", "moBancCaps"),
        ("Thummel v. King, 570 S.W.2d 679 (Mo. App. 2020)", "moAppDistrict"),
        ("Sec. 537.080 RSMo", "moStatuteForm"),
    ];
    for (cite, rule) in cases {
        let mut one = law();
        insert(&mut one, cite);
        let f = check(&mut one, "missouri", false);
        assert!(has_rule(&f, rule), "expected {rule} for `{cite}`: {f:?}");
    }
}

#[test]
fn federal_good_citations_have_no_findings() {
    let mut s = law();
    for cite in [
        "Brown v. Board, 347 U.S. 483 (1954)",
        "Roe v. Wade, 410 U.S. 113 (1973)",
        "Smith v. Jones, 567 F.3d 890 (8th Cir. 2009)",
        "Doe v. Roe, 123 F. Supp. 2d 456 (E.D. Mo. 2005)",
        "42 U.S.C. § 1983",
        "Fed. R. App. P. 32(a)",
        "Fed. R. Civ. P. 12(b)(6)",
    ] {
        insert(&mut s, cite);
        insert(&mut s, "\r");
    }
    let f = check(&mut s, "federal", false);
    assert!(f.is_empty(), "expected no findings: {f:?}");
}

#[test]
fn federal_bad_citations_are_reported() {
    let cases: [(&str, &str); 5] = [
        ("Smith v. Jones, 567 F3d 890 (8th Cir. 2009)", "federalReporterWrong"),
        ("Smith v. Jones, 567 F.3d 890", "federalCourtParen"),
        ("42 U.S.C. 1983", "uscSection"),
        ("Fed R App P 32(a)", "fedRuleApp"),
        ("Fed. R. Civ. P 12", "fedRuleCiv"),
    ];
    for (cite, rule) in cases {
        let mut s = law();
        insert(&mut s, cite);
        let f = check(&mut s, "federal", false);
        assert!(has_rule(&f, rule), "expected {rule} for `{cite}`: {f:?}");
    }
}

#[test]
fn bundled_profiles_reference_citation_packs() {
    let st = store();
    let mo = st.get(MISSOURI).expect("missouri");
    let fed = st.get(FEDERAL).expect("federal");
    assert_eq!(mo.citation_pack.as_deref(), Some("missouri"));
    assert_eq!(fed.citation_pack.as_deref(), Some("federal"));
}

#[test]
fn citation_check_uses_profile_pack_when_applied() {
    let mut s = apply_session(MISSOURI, "brief");
    insert(&mut s, "Thummel v. King, 570 S.W.2d 679");
    let r = run(&mut s, "law.citations.check", json!({}));
    assert_eq!(r["pack"], json!("missouri"));
    assert!(has_rule(r["findings"].as_array().unwrap(), "moCaseParenthetical"));
}

#[test]
fn citation_findings_show_through_proofing() {
    let mut s = apply_session(MISSOURI, "brief");
    insert(&mut s, "Thummel v. King, 570 S.W.2d 679 (Mo. Banc 1978)");
    run(&mut s, "law.citations.check", json!({"pack": "missouri"}));
    let issues = run(&mut s, "review.issues", json!({}));
    assert!(issues.as_array().is_some_and(|a| a.iter().any(|i| i["kind"] == "lint" && i["checkId"] == "moBancCaps")));
    let lint = run(&mut s, "review.lintFindings", json!({}));
    assert!(lint.as_array().is_some_and(|a| a.iter().any(|i| i["checkId"] == "moBancCaps")));
}

#[test]
fn citation_check_is_pure_and_unknown_pack_fails() {
    let mut s = law();
    insert(&mut s, "text");
    let before = serde_json::to_string(&s.doc).expect("serialize");
    run(&mut s, "law.citations.check", json!({"pack": "missouri", "store": false}));
    let after = serde_json::to_string(&s.doc).expect("serialize");
    assert_eq!(before, after);
    let e = err(&mut s, "law.citations.check", json!({"pack": "no-such-pack"})).to_string();
    assert!(e.contains("no citation pack"), "{e}");
    assert!(specs().iter().any(|c| c.id == "law.citations.check"));
}

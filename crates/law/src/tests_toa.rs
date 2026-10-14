//! Table of authorities: grouped entries, PAGEREF page lines, rebuild and undo.

use serde_json::json;
use wordcraft_doc::StoryRef;
use wordcraft_doc::para::InlineObject;

use crate::tests::{MISSOURI, law, roundtrip, run};

fn field_instrs(doc: &wordcraft_doc::Document) -> Vec<String> {
    let mut out = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        for o in &doc.para(StoryRef::Body, &path).unwrap().objects {
            if let InlineObject::Field { instr, .. } = o {
                out.push(instr.clone());
            }
        }
    }
    out
}

fn brief_with_citations(s: &mut wordcraft_engine::Session) {
    run(s, "law.profiles.apply", json!({"id": MISSOURI, "doc": "brief"}));
    run(s, "text.insert", json!({"text": "Table of Authorities"}));
    run(s, "styles.apply", json!({"style": "Heading"}));
    run(s, "text.newParagraph", json!({}));
    let body = "Thummel v. King, 570 S.W.2d 679 (Mo. banc 1978). ";
    run(s, "text.insert", json!({"text": body}));
    run(s, "text.newParagraph", json!({}));
    run(s, "text.insert", json!({"text": "Section 537.080, RSMo 2016. "}));
    run(s, "text.newParagraph", json!({}));
    run(s, "text.insert", json!({"text": "Rule 84.04(d). "}));
    run(s, "text.newParagraph", json!({}));
    run(s, "text.insert", json!({"text": "Id. at 680. "}));
}

#[test]
fn toa_lists_grouped_sorted_citations_with_pageref_fields() {
    let mut s = law();
    brief_with_citations(&mut s);
    run(&mut s, "law.toa.build", json!({}));
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(text.contains("Section 537.080, RSMo 2016"), "statute paragraph must remain after the first build");
    assert!(text.contains("Id. at 680"), "short-form cite must remain in the brief body");
    let cases = text.find("Cases").expect("Cases group");
    let statutes = text.find("Statutes").expect("Statutes group");
    let rules = text.find("Rules").expect("Rules group");
    assert!(cases < statutes && statutes < rules, "groups must appear in order: {text}");
    let toa_region = text.split("Jurisdictional Statement").next().unwrap_or(&text);
    assert_eq!(toa_region.matches("Thummel v. King, 570 S.W.2d 679 (Mo. banc 1978)").count(), 1, "Id. must merge with the case in the table");
    let instrs = field_instrs(&s.doc);
    assert!(instrs.iter().filter(|i| i.starts_with("PAGEREF _LawToa")).count() >= 3, "{instrs:?}");
    let layout = s.layout();
    let case_fields = instrs.iter().filter(|i| i.starts_with("PAGEREF _LawToa")).collect::<Vec<_>>();
    assert!(!case_fields.is_empty());
    let resolved = layout.pages.len();
    assert!(resolved >= 1, "layout must paginate");
    let doc = roundtrip(&s);
    let rt_instrs = field_instrs(&doc);
    assert!(rt_instrs.iter().any(|i| i.contains("PAGEREF") && i.contains("_LawToa")), "{rt_instrs:?}");
    assert!(doc.bookmarks().iter().any(|(n, _)| n.starts_with("_LawToa")), "source bookmarks survive round trip");
}

#[test]
fn toa_rebuild_replaces_the_previous_table_and_undo_restores_it() {
    let mut s = law();
    brief_with_citations(&mut s);
    let body_only = s.doc.plain_text(StoryRef::Body);
    run(&mut s, "law.toa.build", json!({}));
    let first_table = s.doc.plain_text(StoryRef::Body);
    let depth = s.undo_depth();
    run(&mut s, "law.toa.build", json!({}));
    let rebuilt = s.doc.plain_text(StoryRef::Body);
    assert!(rebuilt.contains("Cases") && rebuilt.contains("570 S.W.2d 679"), "rebuild keeps the table");
    assert!(rebuilt.contains("Section 537.080"), "rebuild keeps statute entries");
    assert_eq!(s.undo_depth(), depth + 1);
    s.run("edit.undo", &json!({})).expect("undo rebuild");
    assert_eq!(s.doc.plain_text(StoryRef::Body), first_table, "undo rebuild restores the first table");
    s.run("edit.undo", &json!({})).expect("undo build");
    assert_eq!(s.doc.plain_text(StoryRef::Body), body_only, "undo removes the table");
}

#[test]
fn toa_passim_uses_profile_default_and_can_be_disabled() {
    let mut s = law();
    brief_with_citations(&mut s);
    for _ in 0..4 {
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": "\u{000c}"}));
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": "Id. at 680. "}));
    }
    run(&mut s, "law.toa.build", json!({}));
    let with_passim = s.doc.plain_text(StoryRef::Body);
    assert!(with_passim.contains("passim"), "profile default enables passim for five pages");
    run(&mut s, "law.toa.build", json!({"passim": false}));
    let no_passim = s.doc.plain_text(StoryRef::Body);
    let case_line = no_passim.lines().find(|l| l.contains("570 S.W.2d 679")).expect("case line in table");
    assert!(!case_line.contains("passim"), "passim can be turned off per build: {case_line}");
    let instrs = field_instrs(&s.doc);
    assert!(instrs.iter().filter(|i| i.contains("PAGEREF")).count() >= 5, "every occurrence keeps a PAGEREF");
}

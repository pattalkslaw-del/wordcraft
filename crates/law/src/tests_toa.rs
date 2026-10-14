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
    let before = s.doc.body.len();
    run(&mut s, "law.toa.build", json!({}));
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(text.contains("Cases"), "{text}");
    assert!(text.contains("Statutes"), "{text}");
    assert!(text.contains("Rules"), "{text}");
    assert!(text.contains("Thummel v. King, 570 S.W.2d 679 (Mo. banc 1978)"), "{text}");
    assert!(text.contains("Section 537.080, RSMo"), "{text}");
    assert!(text.contains("Rule 84.04(d)"), "{text}");
    let instrs = field_instrs(&s.doc);
    assert!(instrs.iter().any(|i| i.starts_with("PAGEREF _LawToa")), "{instrs:?}");
    assert!(s.doc.body.len() > before);
    s.layout();
    let doc = roundtrip(&s);
    let rt_instrs = field_instrs(&doc);
    assert!(rt_instrs.iter().any(|i| i.contains("PAGEREF") && i.contains("_LawToa")), "{rt_instrs:?}");
}

#[test]
fn toa_rebuild_replaces_the_previous_table_and_undo_restores_it() {
    let mut s = law();
    brief_with_citations(&mut s);
    let body_only = s.doc.plain_text(StoryRef::Body);
    run(&mut s, "law.toa.build", json!({}));
    let depth = s.undo_depth();
    run(&mut s, "law.toa.build", json!({}));
    let rebuilt = s.doc.plain_text(StoryRef::Body);
    assert!(rebuilt.contains("Cases") && rebuilt.contains("570 S.W.2d 679"), "rebuild keeps the table");
    assert_eq!(s.undo_depth(), depth + 1);
    s.run("edit.undo", &json!({})).expect("undo rebuild");
    assert_eq!(s.doc.plain_text(StoryRef::Body), rebuilt, "undo rebuild leaves the first table");
    s.run("edit.undo", &json!({})).expect("undo build");
    assert_eq!(s.doc.plain_text(StoryRef::Body), body_only, "undo removes the table");
}

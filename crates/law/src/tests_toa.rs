//! Table of authorities: grouped entries, PAGEREF page lines, rebuild and undo.

use serde_json::json;
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Block, Path, StoryRef};

use crate::citations::find_entries;
use crate::cmd::pack_by_id;
use crate::tests::{MISSOURI, law, roundtrip, run};
use crate::toa::{PROP_TOA_PASSIM, PROP_TOA_PASSIM_AT};
use crate::{MODULE, install};
use wordcraft_engine::Session;

const CASE_A: &str = "Thummel v. King, 570 S.W.2d 679 (Mo. banc 1978).";
const CASE_B: &str = "Smith v. Jones, 600 S.W.3d 100 (Mo. App. E.D. 2020).";
const CASE_C: &str = "Doe v. Roe, 610 S.W.3d 200 (Mo. App. W.D. 2021).";
const STATUTE: &str = "Section 537.080, RSMo 2016.";
const RULE: &str = "Rule 84.04(d).";

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

/// The text the table of authorities currently occupies (its stored bounds).
fn table_text(s: &wordcraft_engine::Session) -> String {
    let start = s.doc.bookmarks().into_iter().find(|(n, _)| n == "_LawToaTbl").map(|(_, p)| p.path.last());
    let end = wordcraft_engine::cmd::references::bookmark_end_pos(&s.doc, "_LawToaTbl").map(|p| p.path.last());
    match (start, end) {
        (Some(a), Some(b)) if a <= b => {
            (a..=b).filter_map(|i| s.doc.para(StoryRef::Body, &Path::top(i))).map(|p| p.plain_text()).collect::<Vec<_>>().join("\n")
        }
        _ => String::new(),
    }
}

/// The text of a paragraph as it shows, without field results.
fn shown(p: &wordcraft_doc::Paragraph) -> String {
    p.text.chars().filter(|c| *c != OBJ).collect()
}

/// The `(bookmark, cached result)` of every `PAGEREF` field on a table line.
fn pagerefs(s: &wordcraft_engine::Session) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        if !p.props.style.as_deref().is_some_and(|st| st == "Normal") {
            continue;
        }
        for o in &p.objects {
            if let InlineObject::Field { instr, result, .. } = o
                && let Some(name) = instr.strip_prefix("PAGEREF ")
            {
                out.push((name.split_whitespace().next().unwrap_or(name).to_string(), result.clone()));
            }
        }
    }
    out
}

/// Apply the profile and put the cited argument after a Table of Authorities heading, so the
/// generated table lands before every authority.
fn brief_with_citations(s: &mut wordcraft_engine::Session) {
    run(s, "law.profiles.apply", json!({"id": MISSOURI, "doc": "brief"}));
    run(s, "text.insert", json!({"text": "Table of Authorities"}));
    run(s, "styles.apply", json!({"style": "Heading"}));
    for line in [CASE_B, STATUTE, RULE, CASE_A, CASE_A] {
        run(s, "text.newParagraph", json!({}));
        run(s, "text.insert", json!({"text": line}));
    }
}

#[test]
fn toa_lists_grouped_sorted_citations_with_pageref_fields() {
    let mut s = law();
    brief_with_citations(&mut s);
    run(&mut s, "law.toa.build", json!({}));
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(text.contains("Section 537.080, RSMo 2016"), "statute paragraph must remain after the first build");
    assert!(text.contains("Rule 84.04(d)"), "rule paragraph must remain after the first build");

    let table = table_text(&s);
    let cases = table.find("Cases").expect("Cases group");
    let statutes = table.find("Statutes").expect("Statutes group");
    let rules = table.find("Rules").expect("Rules group");
    assert!(cases < statutes && statutes < rules, "groups must appear in order: {table}");

    // Two cases, sorted alphabetically by the citation as written: 570 before 600 even though
    // 600 was cited first.
    let a = table.find("570 S.W.2d 679").expect("case A in the table");
    let b = table.find("600 S.W.3d 100").expect("case B in the table");
    assert!(a < b && a > cases && b < statutes, "cases must be sorted: {table}");
    // The repeated case is one line, so its citation appears exactly once in the table.
    assert_eq!(table.matches("570 S.W.2d 679").count(), 1, "the repeated case must collapse: {table}");
    assert!(table.contains("Section 537.080"), "statute in the table");
    assert!(table.contains("Rule 84.04(d)"), "rule in the table");

    // The body keeps the short-form source citations; the table links them with bookmarks.
    let instrs = field_instrs(&s.doc);
    assert!(instrs.iter().filter(|i| i.starts_with("PAGEREF _LawToa")).count() >= 3, "{instrs:?}");

    // After the fields update, every page number is the page its bookmark is on.
    run(&mut s, "references.updateFields", json!({}));
    let layout = s.layout();
    let page_of = |name: &str| {
        s.doc
            .bookmarks()
            .into_iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, pos)| layout.caret(&pos).and_then(|c| layout.pages.get(c.page)).map(|pg| pg.number))
    };
    for (name, result) in pagerefs(&s) {
        let Some(page) = page_of(&name) else { continue };
        assert_eq!(result, page.to_string(), "PAGEREF {name} must resolve to its page");
    }

    // Pushing the brief onto a later page updates the page numbers.
    let before = pagerefs(&s);
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "text.insert", json!({"text": "\u{000c}"}));
    run(&mut s, "references.updateFields", json!({}));
    let after = pagerefs(&s);
    assert_eq!(before.len(), after.len());
    assert!(
        before.iter().zip(&after).any(|(b, a)| b.0 == a.0 && b.1 != a.1),
        "page numbers must update after pagination changes: {before:?} vs {after:?}"
    );

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
    let first_table = table_text(&s);
    let first_lines = first_table.lines().count();
    let depth = s.undo_depth();
    run(&mut s, "law.toa.build", json!({}));
    let rebuilt = table_text(&s);
    assert_eq!(rebuilt, first_table, "rebuilding with no edits keeps the same table");
    assert_eq!(rebuilt.lines().count(), first_lines, "and the same entry count");
    assert_eq!(s.undo_depth(), depth + 1);
    s.run("edit.undo", &json!({})).expect("undo rebuild");
    assert_eq!(table_text(&s), first_table, "undo rebuild restores the first table");
    s.run("edit.undo", &json!({})).expect("undo build");
    assert_eq!(s.doc.plain_text(StoryRef::Body), body_only, "undo removes the table");
}

/// Regression: rebuilding must use each source citation's *current* position. Creating the source
/// bookmarks before the old table is removed keeps citations that sit after the table — and grow
/// its size — from being bookmarked at stale paths.
#[test]
fn toa_rebuild_keeps_correct_source_positions_when_the_table_grows() {
    let mut s = law();
    run(&mut s, "law.profiles.apply", json!({"id": MISSOURI, "doc": "brief"}));
    run(&mut s, "text.insert", json!({"text": "Table of Authorities"}));
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": CASE_A}));
    run(&mut s, "law.toa.build", json!({}));

    // More authorities after the table make the rebuilt table larger.
    run(&mut s, "caret.docEnd", json!({}));
    for line in [CASE_B, CASE_C, STATUTE] {
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": line}));
    }
    run(&mut s, "law.toa.build", json!({}));

    let table = table_text(&s);
    assert!(table.contains("570 S.W.2d 679") && table.contains("600 S.W.3d 100") && table.contains("610 S.W.3d 200"), "{table}");

    // Every source bookmark still covers a real citation, not the table or another paragraph.
    let mut covered: Vec<String> = Vec::new();
    for (name, _) in s.doc.bookmarks() {
        if !name.starts_with("_LawToa") || name.contains("Tbl") {
            continue;
        }
        let text = wordcraft_engine::cmd::references::bookmark_text(&s.doc, &name).unwrap_or_default();
        assert!(
            text.contains("S.W.2d 679") || text.contains("S.W.3d 100") || text.contains("S.W.3d 200") || text.contains("Section 537.080"),
            "bookmark {name} covers `{text}`, not a citation"
        );
        covered.push(text);
    }
    assert_eq!(covered.len(), 4, "one bookmark per authority: {covered:?}");
}

#[test]
fn toa_passim_uses_profile_default_and_can_be_disabled() {
    let mut s = law();
    run(&mut s, "law.profiles.apply", json!({"id": MISSOURI, "doc": "brief"}));
    run(&mut s, "text.insert", json!({"text": "Table of Authorities"}));
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": CASE_A}));
    for _ in 0..4 {
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": "\u{000c}"}));
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": CASE_A}));
    }
    run(&mut s, "law.toa.build", json!({}));
    assert!(table_text(&s).contains("passim"), "profile default enables passim for five pages");

    // The passim line shows `passim`, never the page numbers, and keeps the references hidden.
    let line = s
        .doc
        .para_paths(StoryRef::Body)
        .into_iter()
        .filter_map(|p| s.doc.para(StoryRef::Body, &p).cloned())
        .find(|p| p.text.contains("\tpassim"))
        .expect("a passim entry line");
    let after_tab = shown(&line).split('\t').nth(1).unwrap_or_default().to_string();
    assert_eq!(after_tab, "passim", "the page numbers must not show: {:?}", shown(&line));
    let hidden: Vec<bool> = line
        .objects
        .iter()
        .filter_map(|o| match o {
            InlineObject::Field { instr, .. } if instr.starts_with("PAGEREF ") => Some(true),
            _ => None,
        })
        .collect();
    assert!(!hidden.is_empty(), "the hidden references are kept");
    for off in line.object_offsets() {
        if matches!(line.object_at(off), Some(InlineObject::Field { instr, .. }) if instr.starts_with("PAGEREF ")) {
            assert_eq!(line.run_ranges().find(|(r, _)| r.contains(&off)).map(|(_, p)| p.hidden), Some(Some(true)), "page field at {off} is hidden");
        }
    }

    // The passim line and its hidden references survive a round trip.
    let rt = roundtrip(&s);
    let rt_line = rt.body.iter().filter_map(|b| b.as_para()).find(|p| p.text.contains("\tpassim")).expect("passim line after round trip");
    assert_eq!(shown(rt_line).split('\t').nth(1), Some("passim"), "passim still shows after reopening");
    for off in rt_line.object_offsets() {
        if matches!(rt_line.object_at(off), Some(InlineObject::Field { instr, .. }) if instr.starts_with("PAGEREF ")) {
            assert_eq!(
                rt_line.run_ranges().find(|(r, _)| r.contains(&off)).map(|(_, p)| p.hidden),
                Some(Some(true)),
                "hidden page field survives the round trip"
            );
        }
    }

    run(&mut s, "law.toa.build", json!({"passim": false}));
    let no_passim = table_text(&s);
    let case_line = no_passim.lines().find(|l| l.contains("570 S.W.2d 679")).expect("case line in table");
    assert!(!case_line.contains("passim"), "passim can be turned off per build: {case_line}");
    let after_tab = case_line.split('\t').nth(1).unwrap_or_default().trim();
    assert!(
        after_tab.split(',').map(str::trim).any(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())),
        "the tab column must list page numbers, not citation text: {after_tab}"
    );

    let doc = roundtrip(&s);
    assert!(field_instrs(&doc).iter().filter(|i| i.contains("PAGEREF")).count() >= 5, "every occurrence keeps a PAGEREF");
}

/// The entry label is the citation as written at its *first* occurrence; a later occurrence never
/// overwrites it.
#[test]
fn repeated_full_citations_keep_the_first_occurrence_label() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "Thummel v. King, 570 S.W.2d 679 (Mo. banc 1978). "}));
    run(&mut s, "text.newParagraph", json!({}));
    // The same reporter with a pin cite is a distinct finding text here.
    run(&mut s, "text.insert", json!({"text": "Thummel v. King, 570 S.W.2d 679, 680 (Mo. banc 1978). "}));
    let pack = pack_by_id("missouri").expect("missouri pack");
    let entries = find_entries(&s.doc, pack, None);
    let first = entries.iter().find(|e| !e.occurrences.is_empty() && e.label.contains("S.W.2d 679")).expect("first case");
    assert!(first.label.starts_with("570 S.W.2d 679"), "the label is the first occurrence: {}", first.label);
    for e in &entries {
        assert!(e.label.starts_with("570 S.W.2d 679"), "no later text overwrites a label: {}", e.label);
        assert!(e.sort_key.starts_with("570 S.W.2d 679"), "the sort key follows the first label: {}", e.sort_key);
    }
}

/// Citations in separate table cells are distinct paragraphs: neither suppresses the other.
#[test]
fn citations_in_separate_cells_are_distinct() {
    let mut s = law();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 2}));
    run(&mut s, "text.insert", json!({"text": "Thummel v. King, 570 S.W.2d 679 (Mo. banc 1978)"}));
    run(&mut s, "text.tab", json!({}));
    run(&mut s, "text.insert", json!({"text": "Smith v. Jones, 600 S.W.3d 100 (Mo. App. E.D. 2020)"}));
    let pack = pack_by_id("missouri").expect("missouri pack");
    let entries = find_entries(&s.doc, pack, None);
    assert!(entries.iter().any(|e| e.label.contains("S.W.2d 679")), "first cell's citation is found: {entries:?}");
    assert!(entries.iter().any(|e| e.label.contains("S.W.3d 100")), "second cell's citation is found: {entries:?}");
}

#[test]
fn toa_docx_round_trip_then_rebuild_and_undo_keep_one_table() {
    let mut s = law();
    brief_with_citations(&mut s);
    run(&mut s, "law.toa.build", json!({}));
    let first = table_text(&s);
    let doc = roundtrip(&s);
    let mut s2 = Session::new(doc);
    s2.prefs.modules.insert(MODULE.into(), true);
    assert_eq!(install(&mut s2), crate::specs().len());
    let depth = s2.undo_depth();
    run(&mut s2, "law.toa.build", json!({}));
    assert_eq!(table_text(&s2), first, "rebuild after reopening the .docx");
    s2.run("edit.undo", &json!({})).expect("undo rebuild");
    assert_eq!(table_text(&s2), first, "undo rebuild restores the reopened table");
    assert_eq!(s2.undo_depth(), depth);
    assert!(s2.doc.bookmarks().into_iter().filter(|(n, _)| *n == "_LawToaTbl").count() == 1, "exactly one table-range bookmark start");
}

#[test]
fn toa_build_at_cursor_uses_the_top_level_body_block() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "lead paragraph"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": CASE_A}));
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [0], "off": 0}}));
    let out = run(&mut s, "law.toa.build", json!({"at": "cursor", "pack": "missouri"}));
    assert_eq!(out.get("start").and_then(|v| v.as_u64()), Some(0), "insert at block 0 when the caret is there");
    assert!(table_text(&s).contains("570 S.W.2d 679"));

    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "before"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    run(&mut s, "text.insert", json!({"text": CASE_A}));
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [2, 0, 0, 0], "off": 0}}));
    let out = run(&mut s, "law.toa.build", json!({"at": "cursor", "pack": "missouri"}));
    assert_eq!(out.get("start").and_then(|v| v.as_u64()), Some(2), "a caret in a table cell uses that table's body block");
}

/// Two cites of the same case on one page collapse to one page reference until an edit separates them.
#[test]
fn toa_same_page_occurrences_gain_a_second_page_without_rebuild() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": CASE_A}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": CASE_A}));
    run(&mut s, "law.toa.build", json!({"pack": "missouri"}));
    let table = table_text(&s);
    let one_page = table.lines().find(|l| l.contains("570 S.W.2d 679")).and_then(|l| l.split('\t').nth(1)).unwrap_or_default().trim();
    assert!(!one_page.contains(','), "one page while both cites share it: {one_page}");

    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [1], "off": 0}}));
    run(&mut s, "text.insert", json!({"text": "\u{000c}"}));
    run(&mut s, "references.updateFields", json!({}));
    let layout = s.layout();
    let source_pages: std::collections::BTreeSet<String> = s
        .doc
        .bookmarks()
        .into_iter()
        .filter(|(n, _)| n.starts_with("_LawToa") && !n.contains("Tbl"))
        .filter_map(|(_, pos)| layout.caret(&pos).and_then(|c| layout.pages.get(c.page)).map(|pg| pg.number.to_string()))
        .collect();
    assert_eq!(source_pages.len(), 2, "the cites must land on two pages: {source_pages:?}");

    let case_marks: Vec<String> =
        s.doc.bookmarks().into_iter().filter_map(|(n, _)| (n.starts_with("_LawToa") && !n.contains("Tbl")).then_some(n)).collect();
    let mut listed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (name, result) in pagerefs(&s) {
        if case_marks.contains(&name) {
            listed.insert(result);
        }
    }
    assert_eq!(listed.len(), 2, "table PAGEREF results must list both pages: {listed:?} in {case_marks:?}");

    let case_line = s
        .doc
        .para_paths(StoryRef::Body)
        .into_iter()
        .filter_map(|path| s.doc.para(StoryRef::Body, &path).cloned())
        .find(|p| p.plain_text().contains("S.W.2d 679") && p.plain_text().contains('\t'))
        .expect("case line in the table");
    let line_text = case_line.plain_text();
    let pages = line_text.split('\t').nth(1).unwrap_or_default().trim();
    let parts: Vec<&str> = pages.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    assert_eq!(parts.len(), 2, "both pages must show in the table line: `{line_text}`");
    assert_ne!(parts[0], parts[1]);
}

/// Page numbers in the built table must match layout immediately (no separate field-update pass).
#[test]
fn toa_build_resolves_page_numbers_against_final_layout() {
    let mut s = law();
    brief_with_citations(&mut s);
    run(&mut s, "law.toa.build", json!({}));
    let layout = s.layout();
    let page_of = |name: &str| {
        s.doc
            .bookmarks()
            .into_iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, pos)| layout.caret(&pos).and_then(|c| layout.pages.get(c.page)).map(|pg| pg.number.to_string()))
    };
    for (name, result) in pagerefs(&s) {
        if let Some(page) = page_of(&name) {
            assert_eq!(result, page, "PAGEREF {name} must match layout right after build");
        }
    }
}

/// Passim is decided from the layout after the table exists, not from a stale pre-insert layout.
#[test]
fn toa_passim_threshold_uses_post_insert_layout() {
    let mut s = law();
    run(&mut s, "law.profiles.apply", json!({"id": MISSOURI, "doc": "brief"}));
    run(&mut s, "text.insert", json!({"text": "Table of Authorities"}));
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": CASE_A}));
    for _ in 0..3 {
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": "\u{000c}"}));
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": CASE_A}));
    }
    run(&mut s, "law.toa.build", json!({}));
    assert!(!table_text(&s).contains("passim"), "four pages must not print passim yet");

    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "\u{000c}"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": CASE_A}));
    run(&mut s, "law.toa.build", json!({}));
    assert!(table_text(&s).contains("passim"), "five pages after rebuild must print passim");
}

/// The exclusion of a generated table uses the top-level body block, so a citation inside a table
/// cell is not dropped because its *local* index happens to fall in the excluded range.
#[test]
fn table_exclusion_uses_the_top_level_block() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "Thummel v. King, 570 S.W.2d 679 (Mo. banc 1978)."}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    run(&mut s, "text.insert", json!({"text": "Smith v. Jones, 600 S.W.3d 100 (Mo. App. E.D. 2020)."}));
    let pack = pack_by_id("missouri").expect("missouri pack");
    // Exclude body block 0 only: the table (a later block) must still be scanned.
    let entries = find_entries(&s.doc, pack, Some((0, 0)));
    assert!(entries.iter().all(|e| !e.label.contains("S.W.2d 679")), "the excluded paragraph is skipped: {entries:?}");
    assert!(entries.iter().any(|e| e.label.contains("S.W.3d 100")), "the table cell is kept: {entries:?}");
}

/// A field update must not destroy the table's stored bounds: the end bookmark sits on the last
/// entry line, which the refresh rebuilds. Rebuilding then still replaces that one table, undo
/// restores it, and both survive a .docx round trip.
#[test]
fn toa_survives_a_field_refresh_then_rebuild_and_undo() {
    let mut s = law();
    brief_with_citations(&mut s);
    run(&mut s, "law.toa.build", json!({}));
    let first = table_text(&s);

    run(&mut s, "references.updateFields", json!({}));
    assert_eq!(s.doc.bookmarks().into_iter().filter(|(n, _)| n == "_LawToaTbl").count(), 1, "the table start survives a refresh");
    assert!(wordcraft_engine::cmd::references::bookmark_end_pos(&s.doc, "_LawToaTbl").is_some(), "the table end survives a refresh");
    assert_eq!(table_text(&s), first, "the refresh keeps the table in place");

    run(&mut s, "law.toa.build", json!({}));
    assert_eq!(table_text(&s), first, "rebuild after a refresh keeps one table");
    assert_eq!(s.doc.bookmarks().into_iter().filter(|(n, _)| n == "_LawToaTbl").count(), 1, "rebuilding did not stack a second table");
    s.run("edit.undo", &json!({})).expect("undo rebuild");
    assert_eq!(table_text(&s), first, "undo restores the refreshed table");

    run(&mut s, "references.updateFields", json!({}));
    let doc = roundtrip(&s);
    let mut s2 = Session::new(doc);
    s2.prefs.modules.insert(MODULE.into(), true);
    assert_eq!(install(&mut s2), crate::specs().len());
    run(&mut s2, "references.updateFields", json!({}));
    run(&mut s2, "law.toa.build", json!({}));
    assert_eq!(table_text(&s2), first, "rebuild after reopen and refresh keeps one table");
    assert_eq!(s2.doc.bookmarks().into_iter().filter(|(n, _)| n == "_LawToaTbl").count(), 1, "one table after reopening the .docx");
    s2.run("edit.undo", &json!({})).expect("undo rebuild");
    assert_eq!(table_text(&s2), first, "undo after reopening restores the refreshed table");
}

/// A TOA entry line with the given cached page results, as `references.updateFields` leaves them.
fn entry_line(pages: &[&str]) -> wordcraft_doc::Paragraph {
    let bms: Vec<String> = (1..=pages.len()).map(|i| format!("_LawToa{i:09}")).collect();
    let results: Vec<String> = pages.iter().map(|p| (*p).to_string()).collect();
    wordcraft_engine::cmd::references::paragraph_pageref_line("Thummel v. King, 570 S.W.2d 679", &bms, false, 5, &results, 400.0, "Normal")
        .expect("an entries line")
}

fn set_entry(s: &mut Session, pages: &[&str]) {
    *s.doc.para_mut(StoryRef::Body, &Path::top(1)).expect("the entry paragraph") = entry_line(pages);
}

fn entry_shown(s: &Session) -> String {
    shown(s.doc.para(StoryRef::Body, &Path::top(1)).expect("the entry paragraph"))
}

/// The refresh recomputes `passim` from the live page count but keeps the *stored* setting: it
/// crosses the threshold in both directions without a rebuild, keeps a custom threshold, and never
/// turns a disabled passim back on.
#[test]
fn toa_refresh_keeps_the_stored_passim_setting_across_threshold_crossings() {
    let mut s = law();
    s.doc.insert_block(StoryRef::Body, &Path::top(1), Block::Para(entry_line(&["1", "2", "3", "4"]))).expect("insert");
    s.doc.set_custom_prop(PROP_TOA_PASSIM, "1");
    s.doc.set_custom_prop(PROP_TOA_PASSIM_AT, "5");

    crate::toa::refresh(&mut s).expect("refresh");
    assert!(!entry_shown(&s).contains("passim"), "four pages under the default threshold five stay numeric");

    // Upward crossing: a fifth distinct page and the line reads passim.
    set_entry(&mut s, &["1", "2", "3", "4", "5"]);
    crate::toa::refresh(&mut s).expect("refresh");
    assert!(entry_shown(&s).contains("passim"), "five pages reach the default threshold");

    // Downward crossing: back under the threshold and the page numbers return.
    set_entry(&mut s, &["1", "2", "3", "4"]);
    crate::toa::refresh(&mut s).expect("refresh");
    assert!(!entry_shown(&s).contains("passim"), "dropping back to four pages restores the numbers");

    // A custom threshold is kept through the refresh, not reset to five.
    s.doc.set_custom_prop(PROP_TOA_PASSIM_AT, "3");
    crate::toa::refresh(&mut s).expect("refresh");
    assert!(entry_shown(&s).contains("passim"), "four pages reach a custom threshold of three");

    set_entry(&mut s, &["1", "2"]);
    crate::toa::refresh(&mut s).expect("refresh");
    assert!(!entry_shown(&s).contains("passim"), "two pages are under a threshold of three");

    // Disabling passim stays disabled however many pages pile up.
    s.doc.set_custom_prop(PROP_TOA_PASSIM, "0");
    set_entry(&mut s, &["1", "2", "3", "4", "5"]);
    crate::toa::refresh(&mut s).expect("refresh");
    assert!(!entry_shown(&s).contains("passim"), "a disabled passim stays off after a refresh");

    // The stored settings survive a .docx round trip: a custom threshold still applies after a
    // reopen, through the real field-update command.
    s.doc.set_custom_prop(PROP_TOA_PASSIM, "1");
    s.doc.set_custom_prop(PROP_TOA_PASSIM_AT, "3");
    set_entry(&mut s, &["1", "2", "3", "4"]);
    crate::toa::refresh(&mut s).expect("refresh");
    assert!(entry_shown(&s).contains("passim"), "four pages reach a threshold of three");
    let doc = roundtrip(&s);
    let mut s2 = Session::new(doc);
    s2.prefs.modules.insert(MODULE.into(), true);
    assert_eq!(install(&mut s2), crate::specs().len());
    run(&mut s2, "references.updateFields", json!({}));
    assert!(
        shown(s2.doc.para(StoryRef::Body, &Path::top(1)).expect("the reopened entry")).contains("passim"),
        "a custom passim threshold survives a .docx round trip"
    );
}

/// Field refresh follows passim across the threshold through the real command, with no rebuild.
#[test]
fn toa_passim_threshold_refreshes_without_rebuild() {
    let mut s = law();
    run(&mut s, "law.profiles.apply", json!({"id": MISSOURI, "doc": "brief"}));
    run(&mut s, "text.insert", json!({"text": "Table of Authorities"}));
    run(&mut s, "styles.apply", json!({"style": "Heading"}));
    // Four page breaks, then five cites: the last two share the fourth page.
    let mut cites = 0;
    while cites < 5 {
        run(&mut s, "text.newParagraph", json!({}));
        if cites > 0 && cites < 4 {
            run(&mut s, "text.insert", json!({"text": "\u{000c}"}));
            run(&mut s, "text.newParagraph", json!({}));
        }
        run(&mut s, "text.insert", json!({"text": CASE_A}));
        cites += 1;
    }
    run(&mut s, "law.toa.build", json!({}));
    assert!(!table_text(&s).contains("passim"), "four distinct pages are under the threshold: {}", table_text(&s));

    // Separate the shared-page cites: a fifth distinct page, and a field update alone turns passim on.
    let last = s.doc.body.len().saturating_sub(1);
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [last], "off": 0}}));
    run(&mut s, "text.pageBreak", json!({}));
    run(&mut s, "references.updateFields", json!({}));
    assert!(table_text(&s).contains("passim"), "five pages after a refresh must print passim: {}", table_text(&s));

    // Removing the separation and refreshing returns to the numbers, again without a rebuild.
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [last], "off": 0}}));
    run(&mut s, "text.delete", json!({}));
    run(&mut s, "references.updateFields", json!({}));
    assert!(!table_text(&s).contains("passim"), "back to four pages, the numbers return: {}", table_text(&s));
}

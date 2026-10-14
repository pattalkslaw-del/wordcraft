//! Legal symbols: commands, shortcuts, spacing and DOCX round-trip.

use crate::MODULE;
use crate::symbols::{PALETTE, fix_sign_spacing_in_str, sign_spacing_space_ranges};
use crate::tests::roundtrip;
use crate::tests::{law, run};
use serde_json::json;
use wordcraft_doc::para::NBSP;
use wordcraft_doc::{Document, Path, Pos, StoryRef};
use wordcraft_engine::{CmdError, Selection, Session};

fn press(s: &mut Session, combo: &str) -> Result<serde_json::Value, CmdError> {
    let id = s.registry.by_shortcut(combo).map(|c| c.id).unwrap_or_else(|| panic!("nothing bound to {combo}"));
    s.run(id, &json!({}))
}

#[test]
fn every_palette_command_is_installed() {
    let s = law();
    for entry in PALETTE {
        assert!(s.registry.get(entry.command).is_some(), "missing {}", entry.command);
    }
}

#[test]
fn every_symbol_command_inserts_its_glyph() {
    let mut s = law();
    for entry in PALETTE {
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, entry.command, json!({}));
        let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
        assert!(text.ends_with(entry.glyph), "{} should insert {}", entry.command, entry.glyph);
    }
}

#[test]
fn law_symbol_shortcuts_do_not_conflict_with_core() {
    let core = wordcraft_engine::cmd::registry();
    let s = law();
    let mut seen = std::collections::HashMap::new();
    for c in s.registry.all() {
        if !c.id.starts_with("law.symbols.") {
            continue;
        }
        for k in c.shortcut.split(" / ").filter(|k| !k.is_empty()) {
            let key = k.to_ascii_lowercase();
            if let Some(other) = core.by_shortcut(k).map(|c| c.id) {
                panic!("{k} is already bound in core to {other}");
            }
            if let Some(other) = seen.insert(key, c.id) {
                panic!("{k} is bound to both {other} and {}", c.id);
            }
        }
    }
    assert_eq!(s.registry.by_shortcut("Mod+Alt+4").map(|c| c.id), Some("law.symbols.section"));
    assert_eq!(s.registry.by_shortcut("Mod+Alt+Shift+4").map(|c| c.id), Some("law.symbols.sections"));
    assert_eq!(s.registry.by_shortcut("Mod+Alt+6").map(|c| c.id), Some("law.symbols.paragraph"));
    assert_eq!(s.registry.by_shortcut("Mod+Alt+Shift+6").map(|c| c.id), Some("law.symbols.paragraphs"));
}

#[test]
fn law_symbol_shortcuts_insert_through_dispatch() {
    let mut s = law();
    press(&mut s, "Mod+Alt+4").unwrap();
    assert_eq!(s.doc.para_at(&s.sel.focus).unwrap().text, "§");
    press(&mut s, "Mod+Alt+Shift+4").unwrap();
    assert!(s.doc.para_at(&s.sel.focus).unwrap().text.ends_with("§§"));
    press(&mut s, "Mod+Alt+6").unwrap();
    assert!(s.doc.para_at(&s.sel.focus).unwrap().text.ends_with("¶"));
    press(&mut s, "Mod+Alt+Shift+6").unwrap();
    assert!(s.doc.para_at(&s.sel.focus).unwrap().text.ends_with("¶¶"));
}

#[test]
fn section_sign_inserts_with_nonbreaking_space_before_a_digit() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "RSMo "}));
    run(&mut s, "law.symbols.section", json!({}));
    for ch in "1.010".chars() {
        run(&mut s, "text.insert", json!({"text": ch.to_string()}));
    }
    let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
    assert_eq!(text, format!("RSMo §{NBSP}1.010"), "{text}");
}

#[test]
fn fix_spacing_command_and_docx_roundtrip() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "See § 1.010 and §§ 2."}));
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
    assert_eq!(text, format!("See §{NBSP}1.010 and §§{NBSP}2."));
    let doc = roundtrip(&s);
    let text = doc.para_at(&wordcraft_doc::Pos::body(0, 0)).unwrap().text.clone();
    assert_eq!(text, format!("See §{NBSP}1.010 and §§{NBSP}2."), "NBSP survives DOCX: {text}");
}

#[test]
fn fix_spacing_undo_restores_the_ordinary_space() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "See § 1"}));
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("See §{NBSP}1"));
    assert!(s.undo(), "the spacing fix is undoable");
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "See § 1");
    assert!(s.redo());
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("See §{NBSP}1"));
}

#[test]
fn fix_spacing_respects_selection_within_one_paragraph() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "A § 1 and B § 2"}));
    let para = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let left = para.text.find("§ 1").unwrap();
    let right = left + "§ 1".len();
    s.sel = Selection { anchor: Pos::body(0, left), focus: Pos::body(0, right) };
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "selection"}));
    let text = s.doc.para_at(&Pos::body(0, 0)).unwrap().text.clone();
    assert_eq!(text, format!("A §{NBSP}1 and B § 2"));
}

#[test]
fn fix_spacing_respects_selection_across_paragraphs() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "§ 2"}));
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(2, 4) };
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "selection"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("§{NBSP}1"));
    assert_eq!(s.doc.para_at(&Pos::body(2, 0)).unwrap().text, format!("§{NBSP}2"));
}

#[test]
fn fix_spacing_preserves_character_formatting() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    let para = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(para.text, format!("§{NBSP}1"));
    assert!(para.runs.iter().any(|r| r.props.bold.is_some_and(|b| b)), "bold run survives spacing fix");
    let doc = roundtrip(&s);
    let para = doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(para.text, format!("§{NBSP}1"));
    assert!(para.runs.iter().any(|r| r.props.bold.is_some_and(|b| b)));
}

#[test]
fn typing_section_uses_nbsp_and_sec_is_unchanged() {
    let mut s = law();
    s.prefs.autocorrect.enabled = true;
    for ch in "§ 1.010".chars() {
        run(&mut s, "text.insert", json!({"text": ch.to_string()}));
    }
    let typed = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
    assert_eq!(typed, format!("§{NBSP}1.010"));
    // `sec.` typed one character at a time must survive the same single-character AutoFormat hook.
    // (Word's own sentence capitalisation is turned off so the test isolates the legal spacing.)
    let mut s2 = law();
    s2.prefs.autocorrect.enabled = true;
    s2.prefs.autocorrect.cap_sentences = false;
    for ch in "sec. 1.010".chars() {
        run(&mut s2, "text.insert", json!({"text": ch.to_string()}));
    }
    let sec = s2.doc.para_at(&s2.sel.focus).unwrap().text.clone();
    assert_eq!(sec, "sec. 1.010");
}

#[test]
fn fix_spacing_maps_the_selection_so_typing_replaces_it() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "selection"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("§{NBSP}1"));
    // The selection still covers the phrase even though the NBSP grew a byte: typing replaces it.
    run(&mut s, "text.insert", json!({"text": "X"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "X", "typing replaces the whole selection");
    // Undo restores the fixed phrase, redo replaces it again.
    assert!(s.undo());
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("§{NBSP}1"));
    assert!(s.redo());
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "X");
}

#[test]
fn fix_spacing_document_scope_keeps_the_caret_after_changed_text() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    // A caret at the end of the paragraph must move past the wider NBSP, not slip inside it.
    s.sel = Selection::caret(Pos::body(0, 4));
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    run(&mut s, "text.insert", json!({"text": "2"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("§{NBSP}12"));
}

#[test]
fn document_spacing_fixes_non_body_stories_too() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    let note = run(&mut s, "references.footnote", json!({"text": "¶ 4"}))["id"].as_u64().expect("note id") as u32;
    let header = run(&mut s, "insert.header", json!({"text": "¶ 2"}))["story"].as_u64().expect("header id") as u32;
    let footer = run(&mut s, "insert.footer", json!({"text": "§ 3"}))["story"].as_u64().expect("footer id") as u32;
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    assert!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text.starts_with(&format!("§{NBSP}1")), "body");
    assert!(s.doc.plain_text(StoryRef::Part(note)).contains(&format!("¶{NBSP}4")), "footnote");
    assert_eq!(s.doc.plain_text(StoryRef::Part(header)), format!("¶{NBSP}2"), "header");
    assert_eq!(s.doc.plain_text(StoryRef::Part(footer)), format!("§{NBSP}3"), "footer");
}

/// Body paragraph 0 and a header paragraph 0 share `Path([0])`, so mapping a replaced space by
/// path alone moves a header caret that this command never edited (a header `§ 1` and a body
/// `§ 1` used to be indistinguishable). The story must match too.
#[test]
fn document_spacing_does_not_move_a_caret_in_another_story_with_the_same_path() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "§ 1"}));
    let header = run(&mut s, "insert.header", json!({"text": "Header text"}))["story"].as_u64().expect("header id") as u32;
    // A caret inside the header, past the body's space offset but at the same path.
    s.sel = Selection::caret(Pos { story: StoryRef::Part(header), path: Path::top(0), off: 4 });
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    run(&mut s, "text.insert", json!({"text": "X"}));
    assert_eq!(s.doc.para(StoryRef::Part(header), &Path::top(0)).unwrap().text, "HeadXer text", "the header caret stays put");
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, format!("§{NBSP}1"), "the body is still fixed");
}

#[test]
fn legal_autoformat_does_not_run_when_law_is_disabled() {
    crate::symbols::register_text_hook();
    let _enabled = law();
    let mut disabled = Session::new(Document::new());
    disabled.prefs.autocorrect.enabled = true;
    assert!(!disabled.module_enabled(MODULE));
    for ch in "§ 1".chars() {
        disabled.run("text.insert", &json!({"text": ch.to_string()})).unwrap();
    }
    let text = disabled.doc.para_at(&disabled.sel.focus).unwrap().text.clone();
    assert_eq!(text, "§ 1");
}

#[test]
fn fix_sign_spacing_string_cases() {
    let (_, ch) = fix_sign_spacing_in_str("¶ 9");
    assert!(ch);
    assert!(sign_spacing_space_ranges("A § 1 B", 0, 3).is_empty());
    assert_eq!(sign_spacing_space_ranges("A § 1 B", 2, 7).len(), 1);
}

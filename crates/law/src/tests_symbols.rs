//! Legal symbols: commands, shortcuts, spacing and DOCX round-trip.

use crate::symbols::{PALETTE, fix_sign_spacing_in_str};
use crate::tests::roundtrip;
use crate::tests::{law, run};
use serde_json::json;
use wordcraft_doc::para::NBSP;

#[test]
fn every_palette_command_is_installed() {
    let s = law();
    for entry in PALETTE {
        assert!(s.registry.get(entry.command).is_some(), "missing {}", entry.command);
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
fn section_sign_inserts_with_nonbreaking_space_before_a_digit() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "RSMo "}));
    run(&mut s, "law.symbols.section", json!({}));
    run(&mut s, "text.insert", json!({"text": "1.010"}));
    let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
    assert!(text.contains(&format!("§{NBSP}1.010")), "{text}");
}

#[test]
fn fix_spacing_command_and_docx_roundtrip() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "See § 1.010 and §§ 2."}));
    run(&mut s, "law.symbols.fixSpacing", json!({"scope": "document"}));
    let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
    assert!(text.contains(&format!("§{NBSP}1.010")), "{text}");
    assert!(text.contains(&format!("§§{NBSP}2.")), "{text}");
    let doc = roundtrip(&s);
    let text = doc.para_at(&wordcraft_doc::Pos::body(0, 0)).unwrap().text.clone();
    assert!(text.contains(NBSP), "NBSP survives DOCX: {text}");
}

#[test]
fn typing_section_uses_nbsp_and_sec_is_unchanged() {
    let mut s = law();
    s.prefs.autocorrect.enabled = true;
    for ch in "§ 1.010".chars() {
        run(&mut s, "text.insert", json!({"text": ch.to_string()}));
    }
    let typed = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
    assert!(typed.contains(NBSP), "{typed}");
    let mut s2 = law();
    s2.prefs.autocorrect.enabled = true;
    run(&mut s2, "text.insert", json!({"text": "sec. 1.010"}));
    let sec = s2.doc.para_at(&s2.sel.focus).unwrap().text.clone();
    assert_eq!(sec, "sec. 1.010");
}

#[test]
fn fix_sign_spacing_string_cases() {
    let (_, ch) = fix_sign_spacing_in_str("¶ 9");
    assert!(ch);
}

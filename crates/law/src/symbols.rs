//! Legal symbols: insert commands, spacing after §/¶, and typing AutoFormat.

use std::sync::Once;

use serde_json::{Value, json};
use wordcraft_doc::para::NBSP;
use wordcraft_doc::{Pos, StoryRef};
use wordcraft_engine::cmd::{delete_selection, sel_result, type_text};
use wordcraft_engine::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

const LOC: &str = "Law › Symbols";

/// One entry in the legal symbols palette (ribbon when LawCraft is on).
pub struct PaletteEntry {
    pub command: &'static str,
    pub label: &'static str,
    pub glyph: &'static str,
}

/// Legal symbols shown in the palette (in order). ⸹ is omitted when the UI font lacks it.
pub const PALETTE: &[PaletteEntry] = &[
    PaletteEntry { command: "law.symbols.section", label: "Section Sign", glyph: "§" },
    PaletteEntry { command: "law.symbols.sections", label: "Section Signs", glyph: "§§" },
    PaletteEntry { command: "law.symbols.paragraph", label: "Paragraph Sign", glyph: "¶" },
    PaletteEntry { command: "law.symbols.paragraphs", label: "Paragraph Signs", glyph: "¶¶" },
    PaletteEntry { command: "law.symbols.copyright", label: "Copyright Sign", glyph: "©" },
    PaletteEntry { command: "law.symbols.registered", label: "Registered Sign", glyph: "®" },
    PaletteEntry { command: "law.symbols.trademark", label: "Trademark Sign", glyph: "™" },
    PaletteEntry { command: "law.symbols.serviceMark", label: "Service Mark", glyph: "℠" },
    PaletteEntry { command: "law.symbols.dagger", label: "Dagger", glyph: "†" },
    PaletteEntry { command: "law.symbols.doubleDagger", label: "Double Dagger", glyph: "‡" },
    PaletteEntry { command: "law.symbols.referenceMark", label: "Reference Mark", glyph: "⸹" },
    PaletteEntry { command: "law.symbols.enDash", label: "En Dash", glyph: "–" },
    PaletteEntry { command: "law.symbols.emDash", label: "Em Dash", glyph: "—" },
    PaletteEntry { command: "law.symbols.nbsp", label: "Nonbreaking Space", glyph: "\u{00a0}" },
    PaletteEntry { command: "law.symbols.nbHyphen", label: "Nonbreaking Hyphen", glyph: "\u{2011}" },
];

/// Optional reference mark (U+2E39); hosts may hide it when the interface font has no glyph.
pub const REFERENCE_MARK: char = '\u{2e39}';

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("law.symbols.section", "Section Sign", LOC, section).key("Mod+Alt+4"),
        CommandSpec::new("law.symbols.sections", "Section Signs", LOC, sections).key("Mod+Alt+Shift+4"),
        CommandSpec::new("law.symbols.paragraph", "Paragraph Sign", LOC, paragraph).key("Mod+Alt+6"),
        CommandSpec::new("law.symbols.paragraphs", "Paragraph Signs", LOC, paragraphs).key("Mod+Alt+Shift+6"),
        CommandSpec::new("law.symbols.copyright", "Copyright Sign", LOC, copyright),
        CommandSpec::new("law.symbols.registered", "Registered Sign", LOC, registered),
        CommandSpec::new("law.symbols.trademark", "Trademark Sign", LOC, trademark),
        CommandSpec::new("law.symbols.serviceMark", "Service Mark", LOC, service_mark),
        CommandSpec::new("law.symbols.dagger", "Dagger", LOC, dagger),
        CommandSpec::new("law.symbols.doubleDagger", "Double Dagger", LOC, double_dagger),
        CommandSpec::new("law.symbols.referenceMark", "Reference Mark", LOC, reference_mark),
        CommandSpec::new("law.symbols.enDash", "En Dash", LOC, en_dash),
        CommandSpec::new("law.symbols.emDash", "Em Dash", LOC, em_dash),
        CommandSpec::new("law.symbols.nbsp", "Nonbreaking Space", LOC, nbsp),
        CommandSpec::new("law.symbols.nbHyphen", "Nonbreaking Hyphen", LOC, nb_hyphen),
        CommandSpec::new("law.symbols.fixSpacing", "Fix Legal Sign Spacing", LOC, fix_spacing)
            .params(r#"{"scope"?: "selection" | "document"} — replace an ordinary space after § or ¶ before a digit with a non-breaking space; default is the selection, or the whole document when the selection is empty"#)
            .pure(),
    ]
}

fn section(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "§")
}
fn sections(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "§§")
}
fn paragraph(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "¶")
}
fn paragraphs(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "¶¶")
}
fn copyright(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "©")
}
fn registered(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "®")
}
fn trademark(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "™")
}
fn service_mark(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "℠")
}
fn dagger(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "†")
}
fn double_dagger(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "‡")
}
fn reference_mark(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "⸹")
}
fn en_dash(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "–")
}
fn em_dash(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "—")
}
fn nbsp(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "\u{00a0}")
}
fn nb_hyphen(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "\u{2011}")
}

/// Register the typing AutoFormat hook once per process.
pub fn register_text_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| wordcraft_engine::hooks::register_on_text_autoformat(on_text_autoformat));
}

fn insert_text(s: &mut Session, text: &str) -> CmdResult {
    let _ = delete_selection(s)?;
    type_text(s, text)?;
    if needs_nbsp_after_signs(text) {
        maybe_append_sign_nbsp(s)?;
    }
    sel_result(s)
}

fn needs_nbsp_after_signs(text: &str) -> bool {
    text.chars().last().is_some_and(|c| c == '§' || c == '¶')
}

fn maybe_append_sign_nbsp(s: &mut Session) -> Result<(), CmdError> {
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return Ok(()) };
    let rest = para.text.get(f.off..).unwrap_or("");
    if rest.is_empty() || rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        let mut b = [0u8; 4];
        type_text(s, NBSP.encode_utf8(&mut b))?;
    }
    Ok(())
}

/// `§ ` before a digit → `§` + NBSP + digit (same for ¶ and doubled signs).
pub fn fix_sign_spacing_in_str(text: &str) -> (String, bool) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '§' || c == '¶' {
            let sign = c;
            let mut j = i;
            while j < chars.len() && chars[j] == sign {
                j += 1;
            }
            if j < chars.len() && chars[j] == ' ' && chars.get(j + 1).is_some_and(|d| d.is_ascii_digit()) {
                out.extend(chars[i..j].iter());
                out.push(NBSP);
                changed = true;
                i = j + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    (out, changed)
}

fn fix_spacing(s: &mut Session, v: &Value) -> CmdResult {
    let scope = p::str(v, "scope");
    let whole = scope == Some("document") || (scope != Some("selection") && s.sel.is_collapsed());
    let (sel_start, sel_end) = if whole {
        (None, None)
    } else {
        let (a, b) = s.sel.ordered();
        (Some(a), Some(b))
    };
    let mut fixes = 0usize;
    for path in s.doc.para_paths(StoryRef::Body) {
        let p = Pos { story: StoryRef::Body, path: path.clone(), off: 0 };
        if let (Some(start), Some(end)) = (&sel_start, &sel_end)
            && !para_overlaps_selection(&p, start, end)
        {
            continue;
        }
        let para = s.doc.para(StoryRef::Body, &path).cloned();
        let Some(para) = para else { continue };
        let (fixed, changed) = fix_sign_spacing_in_str(&para.text);
        if !changed {
            continue;
        }
        let para = s.doc.para_mut(StoryRef::Body, &path)?;
        para.text = fixed;
        para.touch();
        fixes += 1;
    }
    Ok(json!({"fixes": fixes}))
}

fn para_overlaps_selection(para_pos: &Pos, start: &Pos, end: &Pos) -> bool {
    if para_pos.story != start.story || para_pos.path != start.path {
        return false;
    }
    // Same paragraph as either end of the selection.
    start.path == para_pos.path || end.path == para_pos.path
}

/// After a digit is typed, turn `§ 1` into `§` NBSP `1`.
fn on_text_autoformat(s: &mut Session, text: &str) -> Result<bool, CmdError> {
    if !s.prefs.autocorrect.enabled {
        return Ok(false);
    }
    let ch = text.chars().next().unwrap_or('\0');
    if !ch.is_ascii_digit() {
        return Ok(false);
    }
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f).cloned() else { return Ok(false) };
    let off = f.off;
    if off == 0 {
        return Ok(false);
    }
    let bytes = para.text.as_bytes();
    let digit_start = off.saturating_sub(ch.len_utf8());
    if digit_start >= bytes.len() {
        return Ok(false);
    }
    let space_end = digit_start;
    if space_end == 0 {
        return Ok(false);
    }
    let space_start = space_end.saturating_sub(1);
    if para.text.get(space_start..space_end) != Some(" ") {
        return Ok(false);
    }
    let before_space = para.text.get(..space_start).unwrap_or("");
    let run = sign_run_end(before_space);
    if run == 0 {
        return Ok(false);
    }
    let para = s.doc.para_mut(f.story, &f.path)?;
    let props = para.props_of_char(space_start).clone();
    para.delete(space_start, space_end)?;
    para.insert_text(space_start, NBSP.encode_utf8(&mut [0u8; 4]), &props)?;
    s.sel = Selection::caret(f);
    Ok(true)
}

fn sign_run_end(before_space: &str) -> usize {
    let mut n = 0usize;
    for c in before_space.chars().rev() {
        if c == '§' || c == '¶' {
            n += c.len_utf8();
        } else {
            break;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wordcraft_doc::Document;
    use wordcraft_engine::Session;

    use crate::{MODULE, install};

    fn law() -> Session {
        let mut s = Session::new(Document::new());
        s.prefs.modules.insert(MODULE.into(), true);
        install(&mut s);
        s
    }

    #[test]
    fn fix_sign_spacing_in_string() {
        let (t, ch) = fix_sign_spacing_in_str("§ 1.010");
        assert!(ch);
        assert_eq!(t, format!("§{NBSP}1.010"));
        let (t, ch) = fix_sign_spacing_in_str("§§ 12");
        assert!(ch);
        assert_eq!(t, format!("§§{NBSP}12"));
        let (t, ch) = fix_sign_spacing_in_str("sec. 1");
        assert!(!ch);
        assert_eq!(t, "sec. 1");
    }

    #[test]
    fn typed_section_spacing_undo_restores_normal_space() {
        register_text_hook();
        let mut s = law();
        s.run("text.insert", &json!({"text": "§"})).unwrap();
        s.run("text.insert", &json!({"text": " "})).unwrap();
        s.run("text.insert", &json!({"text": "1"})).unwrap();
        let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
        assert!(text.contains(NBSP), "expected NBSP: {text:?}");
        assert!(s.undo());
        let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
        assert!(text.contains("§ 1"), "undo restores the ordinary space: {text:?}");
    }
}

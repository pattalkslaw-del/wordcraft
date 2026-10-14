//! Table of authorities: build from the citation finder with bookmark + PAGEREF page lines.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::{Value, json};
use wordcraft_doc::collate::sort_key;
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Paragraph, Path, Pos, StoryRef};
use wordcraft_engine::cmd::page;
use wordcraft_engine::cmd::references::{bookmark_end_pos, bookmark_range, paragraph_pageref_line};
use wordcraft_engine::{CmdError, Session, p};

use crate::checks;
use crate::citations::{CitationEntry, find_entries, group_heading};
use crate::profile::{DocSpec, Profile};

const PROP_TOA_START_BM: &str = "LAW_TOA_START_BM";
const PROP_TOA_END_BM: &str = "LAW_TOA_END_BM";
/// The passim setting the table was last built with, kept independently of what the entries
/// currently display so a field refresh never changes the setting out from under the document.
pub(crate) const PROP_TOA_PASSIM: &str = "LAW_TOA_PASSIM";
pub(crate) const PROP_TOA_PASSIM_AT: &str = "LAW_TOA_PASSIM_AT";
const BOOKMARK_PREFIX: &str = "_LawToa";
const BOOKMARK_TBL: &str = "_LawToaTbl";
const DEFAULT_PASSIM_AT: u32 = 5;

/// Build or rebuild the table of authorities at the caret or at the profile's TOA section.
pub fn build(s: &mut Session, v: &Value) -> Result<Value, CmdError> {
    let pack_id = pack_id(s, v)?;
    let pack = crate::cmd::pack_by_id(&pack_id)?;
    let (profile, doc_type) = applied_profile(s, v)?;
    let spec = profile.as_ref().and_then(|p| doc_type.as_ref().and_then(|d| p.docs.get(d)));
    let passim = p::bool(v, "passim").unwrap_or_else(|| spec.and_then(|d| d.toa_passim).unwrap_or(true));
    let passim_at = p::u64(v, "passimAt").map(|n| n.clamp(1, 99) as u32).or_else(|| spec.and_then(|d| d.toa_passim_at)).unwrap_or(DEFAULT_PASSIM_AT);

    let (mut insert_at, replace) = insert_plan(s, spec, v)?;
    if let Some((start, end)) = replace
        && !validated_table_range(s, start, end)
    {
        return Err(CmdError::Failed("stored table of authorities bounds are invalid".into()));
    }
    let entries = find_entries(&s.doc, pack, replace);

    // Source bookmarks are created while the old table is still in the document, so every
    // occurrence path is current; they then move with their text when its blocks are removed.
    let mut bookmarks: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    let mut occ: Vec<(u32, Pos, Pos)> = entries.iter().flat_map(|e| e.occurrences.iter().map(|o| (e.id, o.start.clone(), o.end.clone()))).collect();
    occ.sort_by_key(|(_, start, _)| std::cmp::Reverse(pos_desc(start)));
    for (entry_id, start, end) in occ {
        let name = bookmark_range(s, &start, &end, BOOKMARK_PREFIX)?;
        bookmarks.entry(entry_id).or_default().push(name);
    }

    if let Some((start, end)) = replace {
        for i in (start..=end).rev() {
            s.doc.remove_block(StoryRef::Body, &Path::top(i))?;
        }
        insert_at = start;
    }

    let width = page::sect(s).text_width();
    let lang = wordcraft_engine::cmd::default_lang(s);
    let (blocks, entry_lines) = table_blocks(&entries, &bookmarks, lang.as_deref(), width, passim_at)?;

    for (k, block) in blocks.iter().enumerate() {
        s.doc.insert_block(StoryRef::Body, &Path::top(insert_at + k), block.clone())?;
    }
    if !blocks.is_empty() {
        let end = insert_at + blocks.len() - 1;
        apply_entry_page_lines(s, insert_at, end, &entry_lines, passim, passim_at, width)?;
        mark_table_bounds(s, insert_at, end)?;
        s.doc.set_custom_prop(PROP_TOA_START_BM, BOOKMARK_TBL);
        s.doc.set_custom_prop(PROP_TOA_END_BM, BOOKMARK_TBL);
        // Record the settings this table was built with: a later field refresh recomputes which
        // entries read `passim` from the live page counts, but keeps the user's setting, not the
        // rendering.
        s.doc.set_custom_prop(PROP_TOA_PASSIM, if passim { "1" } else { "0" });
        s.doc.set_custom_prop(PROP_TOA_PASSIM_AT, &passim_at.to_string());
    }
    s.touch();
    Ok(json!({"pack": pack_id, "entries": entries.len(), "blocks": blocks.len(), "start": insert_at}))
}

fn group_order() -> [&'static str; 5] {
    ["Cases", "Constitutional Provisions", "Statutes", "Rules", "Other Authorities"]
}

/// The table's blocks in group order: a `Heading`-styled group heading followed by one
/// page-reference line per authority, with empty groups omitted. Returns each entry line's
/// `(label, bookmarks)` in the same order for the later page-number pass.
pub(crate) fn table_blocks(
    entries: &[CitationEntry],
    bookmarks: &BTreeMap<u32, Vec<String>>,
    lang: Option<&str>,
    width: f32,
    passim_at: u32,
) -> Result<(Vec<Block>, Vec<(String, Vec<String>)>), CmdError> {
    let mut blocks = Vec::new();
    let mut entry_lines: Vec<(String, Vec<String>)> = Vec::new();
    for group in group_order() {
        let mut in_group: Vec<&CitationEntry> = entries.iter().filter(|e| group_heading(&e.kind) == group).collect();
        in_group.sort_by_key(|e| sort_key(&e.sort_key, lang));
        if in_group.is_empty() {
            continue;
        }
        blocks.push(Block::Para(Paragraph::with_text(group, Default::default()).styled("Heading")));
        for e in in_group {
            let bms = bookmarks.get(&e.id).cloned().unwrap_or_default();
            let para = paragraph_pageref_line(&e.label, &bms, false, passim_at, &[], width, "Normal")?;
            entry_lines.push((e.label.clone(), bms));
            blocks.push(Block::Para(para));
        }
    }
    Ok((blocks, entry_lines))
}

fn pack_id(s: &Session, v: &Value) -> Result<String, CmdError> {
    if let Some(id) = p::str(v, "pack") {
        return Ok(id.to_string());
    }
    let profile_id = s
        .doc
        .custom_prop(checks::PROP_PROFILE)
        .ok_or_else(|| CmdError::Params("`pack` (citation pack id) is required when no court profile is applied".into()))?;
    let store = crate::cmd::store();
    let profile = store.get(profile_id).ok_or_else(|| CmdError::Failed(format!("no court profile `{profile_id}`")))?;
    profile.citation_pack.clone().ok_or_else(|| CmdError::Params(format!("court profile `{profile_id}` has no `citationPack`")))
}

fn applied_profile(s: &Session, v: &Value) -> Result<(Option<Profile>, Option<String>), CmdError> {
    let profile_id = p::str(v, "id").map(str::to_string).or_else(|| s.doc.custom_prop(checks::PROP_PROFILE).map(str::to_string));
    let doc_type = p::str(v, "doc").map(str::to_string).or_else(|| s.doc.custom_prop(checks::PROP_DOC_TYPE).map(str::to_string));
    let profile = profile_id.and_then(|id| crate::cmd::store().get(&id).cloned());
    Ok((profile, doc_type))
}

fn insert_plan(s: &mut Session, spec: Option<&DocSpec>, v: &Value) -> Result<(usize, Option<(usize, usize)>), CmdError> {
    let replace = stored_table_range(s);
    if let Some((start, _)) = replace {
        return Ok((start, replace));
    }
    if p::str(v, "at") == Some("cursor") {
        return Ok((body_block_at_caret(s)?, None));
    }
    if let Some(spec) = spec
        && let Some(toa) = checks::section_body_index(&s.doc, spec, "tableOfAuthorities")
    {
        return Ok((toa + 1, None));
    }
    Ok((body_block_at_caret(s)?, None))
}

/// Where a cursor-positioned table goes: the top-level body block the caret is in. At the start of
/// a paragraph the table is inserted before it; past the start the paragraph is split at the caret
/// so the table lands where the caret is, and at the end of a paragraph it is inserted after it. A
/// caret inside a table or another nested container uses its top-level block.
fn body_block_at_caret(s: &mut Session) -> Result<usize, CmdError> {
    let focus = s.sel.focus.clone();
    if focus.story != StoryRef::Body {
        return Err(CmdError::Disabled("place the caret in the document body".into()));
    }
    let i = focus.path.0.first().map(|b| *b as usize).ok_or_else(|| CmdError::Failed("caret path is invalid".into()))?;
    if focus.path.depth() > 1 {
        return Ok(i);
    }
    if focus.off == 0 {
        return Ok(i);
    }
    let len = s.doc.para(StoryRef::Body, &focus.path).map(|p| p.len()).unwrap_or(0);
    if focus.off >= len {
        return Ok(i + 1);
    }
    s.doc.split_paragraph(&focus)?;
    Ok(i + 1)
}

fn stored_table_range(s: &Session) -> Option<(usize, usize)> {
    let start_name = s.doc.custom_prop(PROP_TOA_START_BM)?;
    let end_name = s.doc.custom_prop(PROP_TOA_END_BM)?;
    if start_name != BOOKMARK_TBL || end_name != BOOKMARK_TBL {
        return None;
    }
    let start = s.doc.bookmarks().into_iter().find(|(n, _)| n == BOOKMARK_TBL).map(|(_, p)| p.path.last());
    let end = bookmark_end_pos(&s.doc, BOOKMARK_TBL).map(|p| p.path.last());
    match (start, end) {
        (Some(a), Some(b)) if a <= b => Some((a, b)),
        _ => None,
    }
}

fn validated_table_range(s: &Session, start: usize, end: usize) -> bool {
    if end < start || end >= s.doc.body.len() {
        return false;
    }
    for i in start..=end {
        let Some(para) = s.doc.para(StoryRef::Body, &Path::top(i)) else {
            return false;
        };
        let style = para.props.style.as_deref().unwrap_or("Normal");
        if i == start {
            if !para.plain_text().trim().is_empty() && style != "Heading" && style != "Normal" {
                return false;
            }
        } else if style != "Normal" && style != "Heading" {
            return false;
        }
    }
    s.doc
        .para(StoryRef::Body, &Path::top(start))
        .is_some_and(|p| p.objects.iter().any(|o| matches!(o, InlineObject::BookmarkStart { name } if name == BOOKMARK_TBL)))
        && bookmark_end_pos(&s.doc, BOOKMARK_TBL).is_some_and(|p| p.path.last() == end)
}

fn mark_table_bounds(s: &mut Session, start: usize, end: usize) -> Result<(), CmdError> {
    let path_start = Path::top(start);
    let path_end = Path::top(end);
    let props = CharProps::default();
    let pos_start = Pos { story: StoryRef::Body, path: path_start.clone(), off: 0 };
    s.doc.insert_object(&pos_start, InlineObject::BookmarkStart { name: BOOKMARK_TBL.into() }, &props)?;
    let end_off = s.doc.para(StoryRef::Body, &path_end).map(|p| p.len()).unwrap_or(0);
    let pos_end = Pos { story: StoryRef::Body, path: path_end, off: end_off };
    s.doc.insert_object(&pos_end, InlineObject::BookmarkEnd { name: BOOKMARK_TBL.into() }, &props)?;
    Ok(())
}

fn apply_entry_page_lines(
    s: &mut Session,
    start: usize,
    end: usize,
    entry_lines: &[(String, Vec<String>)],
    passim: bool,
    passim_at: u32,
    width: f32,
) -> Result<(), CmdError> {
    const MAX_ROUNDS: usize = 8;
    let mut prev: Option<Vec<Vec<String>>> = None;
    for _ in 0..MAX_ROUNDS {
        let snapshot = apply_entry_page_lines_once(s, start, end, entry_lines, passim, passim_at, width)?;
        if prev.as_ref() == Some(&snapshot) {
            break;
        }
        prev = Some(snapshot);
    }
    Ok(())
}

fn apply_entry_page_lines_once(
    s: &mut Session,
    start: usize,
    end: usize,
    entry_lines: &[(String, Vec<String>)],
    passim: bool,
    passim_at: u32,
    width: f32,
) -> Result<Vec<Vec<String>>, CmdError> {
    let layout = s.layout();
    let mut page_by_bookmark: BTreeMap<String, String> = BTreeMap::new();
    for name in entry_lines.iter().flat_map(|(_, bms)| bms) {
        if page_by_bookmark.contains_key(name) {
            continue;
        }
        let pos = s.doc.bookmarks().into_iter().find(|(n, _)| n == name).map(|(_, p)| p);
        let page =
            pos.and_then(|p| layout.caret(&p).and_then(|c| layout.pages.get(c.page)).map(|pg| pg.number.to_string())).unwrap_or_else(|| "1".into());
        page_by_bookmark.insert(name.clone(), page);
    }
    let mut snapshot: Vec<Vec<String>> = Vec::new();
    let mut line = 0usize;
    for i in start..=end {
        let path = Path::top(i);
        let Some(para) = s.doc.para(StoryRef::Body, &path) else { continue };
        if para.props.style.as_deref() == Some("Heading") {
            continue;
        }
        let Some((label, bms)) = entry_lines.get(line) else { break };
        line += 1;
        let pages: Vec<String> = bms.iter().map(|n| page_by_bookmark.get(n).cloned().unwrap_or_else(|| "1".into())).collect();
        snapshot.push(pages.clone());
        let updated = paragraph_pageref_line(label, bms, passim, passim_at, &pages, width, "Normal")?;
        *s.doc.para_mut(StoryRef::Body, &path)? = updated;
    }
    Ok(snapshot)
}

fn pos_desc(start: &Pos) -> (i64, usize, usize) {
    let story = match start.story {
        StoryRef::Body => 0i64,
        StoryRef::Part(id) => id as i64 + 1,
    };
    (story, start.path.last(), start.off)
}

/// Register the table's page-reference refresh with the engine, once per process. The engine runs
/// it after field results change (`references.updateFields`); the table itself knows nothing about
/// fields, so the hook lives here, not in core.
pub fn register_field_hook() {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    if REGISTERED.set(()).is_ok() {
        wordcraft_engine::hooks::register_after_field_update(refresh);
    }
}

/// The passim setting the document's table was built with: enabled by default, five pages.
fn stored_passim(s: &Session) -> bool {
    s.doc.custom_prop(PROP_TOA_PASSIM).map(|v| v != "0").unwrap_or(true)
}

fn stored_passim_at(s: &Session) -> u32 {
    s.doc.custom_prop(PROP_TOA_PASSIM_AT).and_then(|v| v.parse::<u32>().ok()).filter(|n| (1..=99).contains(n)).unwrap_or(DEFAULT_PASSIM_AT)
}

/// After field results change, redraw every table entry line from its cached page numbers so the
/// distinct pages shown, and whether the line reads `passim`, stay current. The entry's paragraph
/// is rebuilt in place: its paragraph formatting and mark are kept, and every inline object the
/// page-reference builder does not own (the table's boundary bookmarks) is reinserted at its
/// offset with its own character formatting. The rebuilt page-reference fields and citation text
/// take the builder's default character formatting. Keeping the objects is what keeps the stored
/// table range, so rebuilding still replaces it.
pub fn refresh(s: &mut Session) -> Result<(), CmdError> {
    let passim = stored_passim(s);
    let passim_at = stored_passim_at(s);
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let pairs = entry_pageref_fields(p);
        if pairs.is_empty() {
            continue;
        }
        let Some(citation) = pageref_line_citation(p) else { continue };
        let bookmarks: Vec<String> = pairs.iter().map(|(n, _)| n.clone()).collect();
        let results: Vec<String> = pairs.iter().map(|(_, r)| r.clone()).collect();
        let width = p.props.tabs.as_ref().and_then(|t| t.first()).map(|t| t.pos + 0.5).unwrap_or(400.0);
        let style = p.props.style.clone().unwrap_or_else(|| "Normal".into());
        // Everything the page-reference builder does not own: the table's boundary bookmarks and
        // any other inline object, kept at its original offset and with its own formatting.
        let kept_props = p.props.clone();
        let kept_mark = p.mark.clone();
        let kept_section = p.section.clone();
        let kept: Vec<(usize, InlineObject, CharProps)> = p
            .object_offsets()
            .into_iter()
            .filter_map(|off| p.object_at(off).map(|o| (off, o, p.props_of_char(off))))
            .filter(|(_, o, _)| !is_entry_pageref(o))
            .map(|(off, o, props)| (off, o.clone(), props.clone()))
            .collect();
        let mut fresh = paragraph_pageref_line(&citation, &bookmarks, passim, passim_at, &results, width, &style)?;
        let built_tabs = fresh.props.tabs.clone();
        fresh.props = kept_props;
        if fresh.props.tabs.is_none() {
            fresh.props.tabs = built_tabs;
        }
        fresh.mark = kept_mark;
        fresh.section = kept_section;
        for (off, obj, props) in kept {
            // Keep the object where it was, as near as the rebuilt line allows; an offset inside a
            // rebuilt field placeholder snaps back to a character boundary.
            let mut at = off.min(fresh.len());
            while at > 0 && !fresh.text.is_char_boundary(at) {
                at -= 1;
            }
            fresh.insert_object(at, obj, &props)?;
        }
        *s.doc.para_mut(StoryRef::Body, &path)? = fresh;
    }
    Ok(())
}

/// The article's table entries: `(bookmark, cached result)` for every `_LawToa` `PAGEREF` on the
/// line, in bookmark-number order. Short-form repeats the finder already links share a bookmark.
fn entry_pageref_fields(p: &Paragraph) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for o in &p.objects {
        if let InlineObject::Field { instr, result, .. } = o
            && let Some(rest) = instr.strip_prefix("PAGEREF ")
        {
            let name = rest.split_whitespace().next().unwrap_or(rest);
            if name.starts_with(BOOKMARK_PREFIX) && !name.contains("Tbl") {
                out.push((name.to_string(), result.clone()));
            }
        }
    }
    out.sort_by_key(|(n, _)| n.strip_prefix(BOOKMARK_PREFIX).and_then(|d| d.parse::<u64>().ok()).unwrap_or(0));
    out
}

fn is_entry_pageref(o: &InlineObject) -> bool {
    if let InlineObject::Field { instr, .. } = o
        && let Some(rest) = instr.strip_prefix("PAGEREF ")
    {
        let name = rest.split_whitespace().next().unwrap_or(rest);
        return name.starts_with(BOOKMARK_PREFIX) && !name.contains("Tbl");
    }
    false
}

fn pageref_line_citation(p: &Paragraph) -> Option<String> {
    let shown: String = p.text.chars().filter(|c| *c != OBJ).collect();
    let (cite, _) = shown.split_once('\t')?;
    Some(cite.to_string())
}

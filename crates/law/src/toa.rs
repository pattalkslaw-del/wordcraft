//! Table of authorities: build from the citation finder with bookmark + PAGEREF page lines.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use wordcraft_doc::collate::sort_key;
use wordcraft_doc::para::InlineObject;
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
const BOOKMARK_PREFIX: &str = "_LawToa";
const BOOKMARK_TBL: &str = "_LawToaTbl";

/// Build or rebuild the table of authorities at the caret or at the profile's TOA section.
pub fn build(s: &mut Session, v: &Value) -> Result<Value, CmdError> {
    let pack_id = pack_id(s, v)?;
    let pack = crate::cmd::pack_by_id(&pack_id)?;
    let (profile, doc_type) = applied_profile(s, v)?;
    let spec = profile.as_ref().and_then(|p| doc_type.as_ref().and_then(|d| p.docs.get(d)));
    let passim = p::bool(v, "passim").unwrap_or_else(|| spec.and_then(|d| d.toa_passim).unwrap_or(true));
    let passim_at = p::u64(v, "passimAt").map(|n| n.clamp(1, 99) as u32).or_else(|| spec.and_then(|d| d.toa_passim_at)).unwrap_or(5);

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
    let mut blocks: Vec<Block> = Vec::new();
    let mut entry_lines: Vec<(String, Vec<String>)> = Vec::new();
    for group in group_order() {
        let mut in_group: Vec<&CitationEntry> = entries.iter().filter(|e| group_heading(&e.kind) == group).collect();
        in_group.sort_by_key(|e| sort_key(&e.sort_key, lang.as_deref()));
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

    for (k, block) in blocks.iter().enumerate() {
        s.doc.insert_block(StoryRef::Body, &Path::top(insert_at + k), block.clone())?;
    }
    if !blocks.is_empty() {
        let end = insert_at + blocks.len() - 1;
        apply_entry_page_lines(s, insert_at, end, &entry_lines, passim, passim_at, width)?;
        mark_table_bounds(s, insert_at, end)?;
        s.doc.set_custom_prop(PROP_TOA_START_BM, BOOKMARK_TBL);
        s.doc.set_custom_prop(PROP_TOA_END_BM, BOOKMARK_TBL);
    }
    s.touch();
    Ok(json!({"pack": pack_id, "entries": entries.len(), "blocks": blocks.len(), "start": insert_at}))
}

fn group_order() -> [&'static str; 5] {
    ["Cases", "Constitutional Provisions", "Statutes", "Rules", "Other Authorities"]
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

fn insert_plan(s: &Session, spec: Option<&DocSpec>, v: &Value) -> Result<(usize, Option<(usize, usize)>), CmdError> {
    let replace = stored_table_range(s);
    let insert_at = if let Some((start, _)) = replace {
        start
    } else if p::str(v, "at") == Some("cursor") {
        body_block_at_caret(s)?
    } else if let Some(spec) = spec
        && let Some(toa) = checks::section_body_index(&s.doc, spec, "tableOfAuthorities")
    {
        toa + 1
    } else {
        body_block_at_caret(s)?
    };
    Ok((insert_at, replace))
}

fn body_block_at_caret(s: &Session) -> Result<usize, CmdError> {
    let focus = &s.sel.focus;
    if focus.story != StoryRef::Body {
        return Err(CmdError::Disabled("place the caret in the document body".into()));
    }
    focus.path.0.first().map(|i| *i as usize).ok_or_else(|| CmdError::Failed("caret path is invalid".into()))
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
        let updated = paragraph_pageref_line(label, bms, passim, passim_at, &pages, width, "Normal")?;
        *s.doc.para_mut(StoryRef::Body, &path)? = updated;
    }
    Ok(())
}

fn pos_desc(start: &Pos) -> (i64, usize, usize) {
    let story = match start.story {
        StoryRef::Body => 0i64,
        StoryRef::Part(id) => id as i64 + 1,
    };
    (story, start.path.last(), start.off)
}

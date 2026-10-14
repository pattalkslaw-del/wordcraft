//! Table of authorities: build from the citation finder with bookmark + PAGEREF page lines.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use wordcraft_doc::collate::sort_key;
use wordcraft_doc::{Block, Paragraph, Path, Pos, StoryRef};
use wordcraft_engine::cmd::page;
use wordcraft_engine::cmd::references::{bookmark_range, paragraph_pageref_line};
use wordcraft_engine::{CmdError, Session, p};

use crate::checks;
use crate::citations::{CitationEntry, find_entries, group_heading};
use crate::profile::{DocSpec, Profile};

const PROP_TOA_START: &str = "LAW_TOA_START";
const PROP_TOA_END: &str = "LAW_TOA_END";
const BOOKMARK_PREFIX: &str = "_LawToa";

/// Build or rebuild the table of authorities at the caret or at the profile's TOA section.
pub fn build(s: &mut Session, v: &Value) -> Result<Value, CmdError> {
    let pack_id = pack_id(s, v)?;
    let pack = crate::cmd::pack_by_id(&pack_id)?;
    let passim = p::bool(v, "passim").unwrap_or(true);
    let passim_at = p::u64(v, "passimAt").unwrap_or(5).clamp(1, 99) as u32;
    let (profile, doc_type) = applied_profile(s, v)?;
    let spec = profile.as_ref().and_then(|p| doc_type.as_ref().and_then(|d| p.docs.get(d)));

    let entries = find_entries(&s.doc, pack);
    let mut bookmarks: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    let mut occ: Vec<(u32, Pos, Pos)> = entries.iter().flat_map(|e| e.occurrences.iter().map(|o| (e.id, o.start.clone(), o.end.clone()))).collect();
    occ.sort_by_key(|(_, start, _)| std::cmp::Reverse(pos_desc(start)));
    for (entry_id, start, end) in occ {
        let name = bookmark_range(s, &start, &end, BOOKMARK_PREFIX)?;
        bookmarks.entry(entry_id).or_default().push(name);
    }

    let layout = s.layout();
    let mut page_by_bookmark: BTreeMap<String, String> = BTreeMap::new();
    for name in bookmarks.values().flatten() {
        let pos = s.doc.bookmarks().into_iter().find(|(n, _)| n == name).map(|(_, p)| p);
        let page =
            pos.and_then(|p| layout.caret(&p).and_then(|c| layout.pages.get(c.page)).map(|pg| pg.number.to_string())).unwrap_or_else(|| "1".into());
        page_by_bookmark.insert(name.clone(), page);
    }

    let (mut insert_at, replace) = insert_plan(s, spec, v)?;
    if let Some((start, end)) = replace {
        for i in (start..=end).rev() {
            s.doc.remove_block(StoryRef::Body, &Path::top(i))?;
        }
        insert_at = start;
    }
    let width = page::sect(s).text_width();
    let lang = wordcraft_engine::cmd::default_lang(s);
    let mut blocks: Vec<Block> = Vec::new();
    let groups = group_order();
    for group in groups {
        let mut in_group: Vec<&CitationEntry> = entries.iter().filter(|e| group_heading(&e.kind) == group).collect();
        in_group.sort_by_key(|e| sort_key(&e.sort_key, lang.as_deref()));
        if in_group.is_empty() {
            continue;
        }
        blocks.push(Block::Para(Paragraph::with_text(group, Default::default()).styled("Heading")));
        for e in in_group {
            let bms = bookmarks.get(&e.id).cloned().unwrap_or_default();
            let mut pages: Vec<String> = Vec::new();
            let mut page_bms: Vec<String> = Vec::new();
            for n in &bms {
                let p = page_by_bookmark.get(n).cloned().unwrap_or_else(|| "1".into());
                if !pages.contains(&p) {
                    pages.push(p);
                    page_bms.push(n.clone());
                }
            }
            let para = paragraph_pageref_line(&e.label, &page_bms, passim, passim_at, &pages, width, "Normal")?;
            blocks.push(Block::Para(para));
        }
    }
    for (k, block) in blocks.iter().enumerate() {
        s.doc.insert_block(StoryRef::Body, &Path::top(insert_at + k), block.clone())?;
    }
    if !blocks.is_empty() {
        let end = insert_at + blocks.len() - 1;
        s.doc.set_custom_prop(PROP_TOA_START, &insert_at.to_string());
        s.doc.set_custom_prop(PROP_TOA_END, &end.to_string());
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
    if p::str(v, "at") == Some("cursor") {
        let at = s.sel.focus.path.last();
        let rep = stored_range(s);
        return Ok((at, rep));
    }
    if let Some(spec) = spec
        && let Some(toa) = checks::section_body_index(&s.doc, spec, "tableOfAuthorities")
    {
        let replace = content_range(&s.doc, spec, toa);
        return Ok((toa + 1, replace));
    }
    let at = s.sel.focus.path.last();
    Ok((at, stored_range(s)))
}

fn stored_range(s: &Session) -> Option<(usize, usize)> {
    let start = s.doc.custom_prop(PROP_TOA_START).and_then(|t| t.parse::<usize>().ok());
    let end = s.doc.custom_prop(PROP_TOA_END).and_then(|t| t.parse::<usize>().ok());
    match (start, end) {
        (Some(a), Some(b)) if a <= b => Some((a, b)),
        _ => None,
    }
}

fn content_range(doc: &wordcraft_doc::Document, spec: &DocSpec, toa_heading: usize) -> Option<(usize, usize)> {
    if let (Some(a), Some(b)) =
        (doc.custom_prop(PROP_TOA_START).and_then(|t| t.parse().ok()), doc.custom_prop(PROP_TOA_END).and_then(|t| t.parse().ok()))
        && a > toa_heading
    {
        return Some((a, b));
    }
    let sections = locate_headings(doc, spec);
    let here = sections.iter().position(|(_, idx)| *idx == toa_heading)?;
    let next = sections.get(here + 1).map(|(_, i)| *i).unwrap_or(doc.body.len());
    let start = toa_heading + 1;
    if start >= next {
        return None;
    }
    Some((start, next - 1))
}

fn locate_headings(doc: &wordcraft_doc::Document, spec: &DocSpec) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(para) = doc.para(StoryRef::Body, &path) else { continue };
        let idx = path.last();
        for sec in &spec.sections {
            if para.plain_text().trim().eq_ignore_ascii_case(sec.name.trim()) && !found.iter().any(|(id, _)| id == &sec.id) {
                found.push((sec.id.clone(), idx));
            }
        }
    }
    found
}

fn pos_desc(start: &Pos) -> (i64, usize, usize) {
    let story = match start.story {
        StoryRef::Body => 0i64,
        StoryRef::Part(id) => id as i64 + 1,
    };
    (story, start.path.last(), start.off)
}

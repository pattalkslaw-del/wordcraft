//! Citation finder and form checker (suggestions only; never claims an authority exists).

use std::collections::HashMap;

use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_engine::lint::{Finding, MAX_FINDINGS};

use crate::citation_pack::CompiledPack;

/// One logical authority in the document (first full cite + linked short forms).
#[derive(Clone, Debug, PartialEq)]
pub struct CitationEntry {
    pub id: u32,
    pub kind: String,
    /// Full citation text at first occurrence.
    pub label: String,
    /// Sort key (leading non-adversarial prefixes stripped for cases).
    pub sort_key: String,
    pub occurrences: Vec<CitationOccurrence>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CitationOccurrence {
    pub start: Pos,
    pub end: Pos,
}

struct RawHit {
    kind: String,
    text: String,
    start: Pos,
    end: Pos,
    rule_id: String,
}

/// Map a pack rule `kind` to a table-of-authorities group heading.
pub fn group_heading(kind: &str) -> &'static str {
    match kind {
        "case" => "Cases",
        "constitutional" | "constitution" => "Constitutional Provisions",
        "statute" => "Statutes",
        "rule" => "Rules",
        _ => "Other Authorities",
    }
}

fn sort_key_for_label(pack: &CompiledPack, label: &str) -> String {
    let t = label.trim();
    for prefix in &pack.sort_strip_prefixes {
        if t.starts_with(prefix) {
            return t[prefix.len()..].trim().to_string();
        }
    }
    t.to_string()
}

/// Scan `doc` with `pack` and return authority entries. Paragraphs in `exclude_blocks` (inclusive
/// body block indices) are skipped — used to omit a generated table of authorities.
pub fn find_entries(doc: &Document, pack: &CompiledPack, exclude_blocks: Option<(usize, usize)>) -> Vec<CitationEntry> {
    let mut raw = Vec::new();
    for story in doc.counted_stories() {
        for path in doc.para_paths(story) {
            if story == StoryRef::Body
                && let Some((lo, hi)) = exclude_blocks
                && {
                    let i = path.last();
                    i >= lo && i <= hi
                }
            {
                continue;
            }
            let Some(para) = doc.para(story, &path) else { continue };
            let text = &para.text;
            if text.is_empty() {
                continue;
            }
            for rule in &pack.rules {
                if !rule.finder {
                    continue;
                }
                for m in rule.detect.find_iter(text) {
                    let end = if rule.extend_tail { extend_span(text, m.end()) } else { m.end() };
                    let slice = text.get(m.start()..end).map_or(m.as_str(), |s| s);
                    if rule.valid.as_ref().is_some_and(|v| !v.is_match(slice)) {
                        continue;
                    }
                    let start = Pos { story, path: path.clone(), off: m.start() };
                    let end_pos = Pos { story, path: path.clone(), off: end };
                    raw.push(RawHit { kind: rule.kind.clone(), text: slice.to_string(), start, end: end_pos, rule_id: rule.id.clone() });
                }
            }
        }
    }
    raw = dedupe_overlaps(raw);
    raw.sort_by_key(|h| pos_order(&h.start, &h.end));

    let mut next_id = 1u32;
    let mut key_to_id: HashMap<String, u32> = HashMap::new();
    let mut last_case: Option<u32> = None;
    let mut hits: Vec<(u32, RawHit, bool)> = Vec::new();
    for h in raw {
        let rule = pack.rules.iter().find(|r| r.id == h.rule_id);
        let link_last = rule.and_then(|r| r.link.as_deref()) == Some("lastCase");
        let is_primary = !link_last;
        let id = if link_last {
            last_case.unwrap_or_else(|| {
                let id = next_id;
                next_id += 1;
                id
            })
        } else if let Some(re) = rule.and_then(|r| r.entry_key.as_ref()) {
            let key = re.find(&h.text).map(|m| m.as_str().to_ascii_lowercase()).unwrap_or_else(|| h.text.to_ascii_lowercase());
            *key_to_id.entry(key).or_insert_with(|| {
                let id = next_id;
                next_id += 1;
                id
            })
        } else {
            let key = format!("{}:{}", h.kind, h.text.to_ascii_lowercase());
            *key_to_id.entry(key).or_insert_with(|| {
                let id = next_id;
                next_id += 1;
                id
            })
        };
        if h.kind == "case" && is_primary {
            last_case = Some(id);
        }
        hits.push((id, h, is_primary));
    }

    let mut map: HashMap<u32, (String, String, Vec<CitationOccurrence>)> = HashMap::new();
    for (id, h, is_primary) in hits {
        let slot = map.entry(id).or_insert_with(|| (h.kind.clone(), h.text.trim().to_string(), Vec::new()));
        if is_primary {
            slot.0 = h.kind.clone();
            slot.1 = h.text.trim().to_string();
        }
        slot.2.push(CitationOccurrence { start: h.start, end: h.end });
    }
    let mut out: Vec<CitationEntry> = Vec::new();
    for (id, (kind, label, occurrences)) in map {
        let sort_key = sort_key_for_label(pack, &label);
        out.push(CitationEntry { id, kind, label, sort_key, occurrences });
    }
    out.sort_by_key(|e| e.id);
    out
}

fn pos_order(start: &Pos, _end: &Pos) -> (i64, usize, usize) {
    let story = match start.story {
        StoryRef::Body => 0,
        StoryRef::Part(id) => id as i64 + 1,
    };
    (story, start.path.last(), start.off)
}

fn para_key(story: StoryRef, path: &wordcraft_doc::Path) -> (i64, usize) {
    let story = match story {
        StoryRef::Body => 0,
        StoryRef::Part(id) => id as i64 + 1,
    };
    (story, path.last())
}

fn dedupe_overlaps(hits: Vec<RawHit>) -> Vec<RawHit> {
    let mut hits = hits;
    hits.sort_by(|a, b| b.text.len().cmp(&a.text.len()).then_with(|| pos_order(&a.start, &a.end).cmp(&pos_order(&b.start, &b.end))));
    let mut taken: HashMap<(i64, usize), Vec<(usize, usize)>> = HashMap::new();
    let mut out = Vec::new();
    for h in hits {
        let key = para_key(h.start.story, &h.start.path);
        let ranges = taken.entry(key).or_default();
        if ranges.iter().any(|(a, b)| h.start.off < *b && h.end.off > *a) {
            continue;
        }
        ranges.push((h.start.off, h.end.off));
        out.push(h);
    }
    out.sort_by_key(|h| pos_order(&h.start, &h.end));
    out
}

/// Extend a match through optional pin cites (including page ranges) and a parenthetical so
/// validation sees the full cite.
fn extend_span(text: &str, end: usize) -> usize {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut pos = end.min(len);
    while pos < len {
        let mut i = pos;
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= len {
            pos = i;
            break;
        }
        if bytes[i] == b',' {
            i += 1;
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let digit_start = i;
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == digit_start {
                break;
            }
            pos = extend_page_range(bytes, i);
            continue;
        }
        if bytes[i] == b'(' {
            i += 1;
            while i < len && bytes[i] != b')' {
                i += 1;
            }
            if i < len && bytes[i] == b')' {
                i += 1;
            }
            pos = i;
            break;
        }
        break;
    }
    pos
}

/// After a pin-cite page ends at `from`, consume a `-`, `–` or `—` page range and its end page.
fn extend_page_range(bytes: &[u8], from: usize) -> usize {
    let len = bytes.len();
    let dash_end = if bytes.get(from) == Some(&b'-') {
        from + 1
    } else if bytes.get(from) == Some(&0xE2) && bytes.get(from + 1) == Some(&0x80) && matches!(bytes.get(from + 2), Some(&0x93) | Some(&0x94)) {
        from + 3
    } else {
        return from;
    };
    let mut i = dash_end;
    while i < len && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let digit_start = i;
    while i < len && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digit_start { from } else { i }
}

/// Scan the document with `pack` and return form suggestions.
pub fn check_document(doc: &Document, pack: &CompiledPack) -> Vec<Finding> {
    let mut out = Vec::new();
    for story in doc.counted_stories() {
        for path in doc.para_paths(story) {
            let Some(para) = doc.para(story, &path) else { continue };
            let text = &para.text;
            if text.is_empty() {
                continue;
            }
            for rule in &pack.rules {
                for m in rule.detect.find_iter(text) {
                    let end = if rule.extend_tail { extend_span(text, m.end()) } else { m.end() };
                    let slice = text.get(m.start()..end).map_or(m.as_str(), |s| s);
                    if rule.valid.as_ref().is_some_and(|v| v.is_match(slice)) {
                        continue;
                    }
                    let start = Pos { story, path: path.clone(), off: m.start() };
                    let end_pos = Pos { story, path: path.clone(), off: end };
                    let msg = format!("{} ({})", rule.message, rule.source);
                    out.push(Finding::at(rule.id.clone(), "warning", msg, start, end_pos));
                    if out.len() >= MAX_FINDINGS {
                        return out;
                    }
                }
            }
        }
    }
    out
}

/// Findings as JSON (same shape as court checks).
pub fn findings_json(findings: &[Finding]) -> Vec<serde_json::Value> {
    findings.iter().map(|f| wordcraft_engine::lint::finding_json(f, wordcraft_engine::cmd::pos_json)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citation_pack::bundled_map;
    use serde_json::json;
    use wordcraft_engine::Session;

    fn check_text(pack_id: &str, text: &str) -> Vec<String> {
        let packs = bundled_map();
        let pack = packs.get(pack_id).expect("pack");
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": text})).expect("insert");
        check_document(&s.doc, pack).into_iter().map(|f| f.check_id).collect()
    }

    fn findings(pack_id: &str, text: &str) -> Vec<Finding> {
        let packs = bundled_map();
        let pack = packs.get(pack_id).expect("pack");
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": text})).expect("insert");
        check_document(&s.doc, pack)
    }

    #[test]
    fn extend_pin_without_parenthetical_is_flagged() {
        let ids = check_text("missouri", "Thummel v. King, 570 S.W.2d 679, 680");
        assert!(ids.iter().any(|id| id == "moCaseParenthetical"));
    }

    #[test]
    fn non_adversarial_case_missing_parenthetical_is_flagged() {
        for cite in ["In re Smith, 570 S.W.2d 679", "Ex parte Jones, 600 S.W.3d 100", "State ex rel. Roe, 590 S.W.3d 200"] {
            let ids = check_text("missouri", cite);
            assert!(ids.iter().any(|id| id == "moCaseParenthetical"), "`{cite}` must be flagged: {ids:?}");
        }
    }

    #[test]
    fn non_adversarial_case_with_parenthetical_is_ok() {
        let ids = check_text("missouri", "In re Smith, 570 S.W.2d 679 (Mo. banc 1978)");
        assert!(!ids.iter().any(|id| id == "moCaseParenthetical"), "unexpected: {ids:?}");
    }

    #[test]
    fn pin_page_range_is_checked_as_one_span() {
        let text = "Thummel v. King, 570 S.W.2d 679, 680-681 (Mo. banc 1978)";
        let fs = findings("missouri", text);
        assert!(!fs.iter().any(|f| f.check_id == "moCaseParenthetical"), "{fs:?}");

        let bad = "Thummel v. King, 570 S.W.2d 679, 680-681";
        let fs = findings("missouri", bad);
        let f = fs.iter().find(|f| f.check_id == "moCaseParenthetical").expect("flagged with a pin range");
        let end = f.end.as_ref().expect("span end").off;
        assert_eq!(end, bad.len(), "the span must reach the end of the pin range");
    }

    #[test]
    fn double_space_before_parenthetical_is_ok() {
        let ids = check_text("missouri", "Thummel v. King, 570 S.W.2d 679  (Mo. banc 1978)");
        assert!(!ids.iter().any(|id| id == "moCaseParenthetical"));
    }

    #[test]
    fn mo_banc_without_year_is_flagged() {
        let ids = check_text("missouri", "Thummel v. King, 570 S.W.2d 679 (Mo. banc)");
        assert!(ids.iter().any(|id| id == "moBancMissingYear"));
    }

    #[test]
    fn federal_pin_without_parenthetical_is_flagged() {
        let ids = check_text("federal", "Smith v. Jones, 567 F.3d 890, 891");
        assert!(ids.iter().any(|id| id == "federalCourtParen"));
    }

    #[test]
    fn us_without_parenthetical_is_flagged() {
        let ids = check_text("federal", "Brown v. Board, 347 U.S. 483");
        assert!(ids.iter().any(|id| id == "usReporter"));
    }

    #[test]
    fn s_ct_spacing_without_space_is_flagged() {
        let ids = check_text("federal", "Roe v. Wade, 410 S.Ct. 113 (1973)");
        assert!(ids.iter().any(|id| id == "sCtReporterTight"));
    }
}

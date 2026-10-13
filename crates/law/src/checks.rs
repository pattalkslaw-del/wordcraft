//! Court profile checks: evaluate a profile's `checks` against the open document.

use serde_json::{Value, json};
use wordcraft_doc::props::LineSpacing;
use wordcraft_doc::{Document, Paragraph, Path, Pos, StoryRef};
use wordcraft_engine::lint::{Finding, MAX_FINDINGS};
use wordcraft_engine::{Session, io};

use crate::profile::{CheckSpec, DocSpec, Profile, SectionSpec};
use crate::store::LawError;

/// Custom document properties written by [`crate::apply::apply`].
pub const PROP_PROFILE: &str = "LAW_PROFILE_ID";
pub const PROP_DOC_TYPE: &str = "LAW_DOC_TYPE";

/// Run every check for `doc_type` in `profile` against `doc`. `for_export` runs export-only
/// checks such as `textSearchablePdf`.
pub fn evaluate(doc: &Document, profile: &Profile, doc_type: &str, for_export: bool) -> Result<Vec<Finding>, LawError> {
    let Some(spec) = profile.docs.get(doc_type) else {
        return Err(LawError::Failed(format!(
            "profile `{}` has no document type `{doc_type}` (it has {})",
            profile.id,
            profile.doc_types().join(", ")
        )));
    };
    let sections = locate_sections(doc, spec);
    let mut out = Vec::new();
    for (i, check) in spec.checks.iter().enumerate() {
        let id = check_id(check, i);
        run_one(doc, spec, &sections, check, &id, for_export, &mut out);
        if out.len() >= MAX_FINDINGS {
            break;
        }
    }
    Ok(out)
}

fn check_id(c: &CheckSpec, index: usize) -> String {
    if c.kind.is_empty() { format!("check{index}") } else { c.kind.clone() }
}

fn run_one(doc: &Document, spec: &DocSpec, sections: &[LocatedSection], check: &CheckSpec, id: &str, for_export: bool, out: &mut Vec<Finding>) {
    match check.kind.as_str() {
        "wordLimit" => word_limit(doc, spec, sections, check, id, out),
        "minFontSize" => min_font_size(doc, check, id, out),
        "minMargins" => min_margins(doc, spec, check, id, out),
        "minLineSpacing" => min_line_spacing(doc, spec, check, id, out),
        "noHyperlinks" => no_hyperlinks(doc, id, out),
        "textSearchablePdf" => {
            if for_export {
                text_searchable_pdf(doc, id, out);
            }
        }
        "requiredSections" => required_sections(sections, check, id, out),
        _ => {}
    }
}

struct LocatedSection {
    id: String,
    path: Path,
}

/// Section headings found in document order (by matching profile section names).
fn locate_sections(doc: &Document, spec: &DocSpec) -> Vec<LocatedSection> {
    let mut found = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(para) = doc.para(StoryRef::Body, &path) else { continue };
        for sec in &spec.sections {
            if section_heading_matches(doc, para, sec) && !found.iter().any(|f: &LocatedSection| f.id == sec.id) {
                found.push(LocatedSection { id: sec.id.clone(), path: path.clone() });
            }
        }
    }
    found
}

fn section_heading_matches(_doc: &Document, para: &Paragraph, sec: &SectionSpec) -> bool {
    // A section starts at a paragraph whose visible text is the section heading (Word puts the
    // title on its own line). Style is applied by the profile but headings are identified by
    // text so agents and pasted documents still match.
    para.plain_text().trim().eq_ignore_ascii_case(sec.name.trim())
}

fn para_style_name(doc: &Document, para: &Paragraph) -> Option<String> {
    let id = para.props.style.as_deref()?;
    doc.styles.get(id).map(|s| s.name.clone()).or_else(|| doc.styles.find(id).map(|s| s.name.clone()))
}

fn required_sections(found: &[LocatedSection], check: &CheckSpec, id: &str, out: &mut Vec<Finding>) {
    if check.ids.is_empty() {
        return;
    }
    let have: Vec<&str> = found.iter().map(|s| s.id.as_str()).collect();
    let mut hi = 0;
    for want in &check.ids {
        while hi < have.len() && have[hi] != want {
            hi += 1;
        }
        if hi >= have.len() {
            let source = &check.source;
            out.push(Finding::error(id, format!("Required section `{want}` is missing or out of order (from {source})")));
            return;
        }
        hi += 1;
    }
}

fn word_limit(doc: &Document, spec: &DocSpec, sections: &[LocatedSection], check: &CheckSpec, id: &str, out: &mut Vec<Finding>) {
    let Some(limit) = check.limit else { return };
    let exclude: Vec<&str> = check.exclude_sections.iter().map(String::as_str).collect();
    let words = counted_words(doc, spec, sections, &exclude);
    if words > limit {
        let source = &check.source;
        out.push(Finding::error(id, format!("Word count {words} exceeds the limit of {limit} ({source})")));
    }
}

fn counted_words(doc: &Document, _spec: &DocSpec, sections: &[LocatedSection], exclude: &[&str]) -> u32 {
    let paths = doc.para_paths(StoryRef::Body);
    if paths.is_empty() {
        return 0;
    }
    let mut total = 0u32;
    for (pi, path) in paths.iter().enumerate() {
        let current = section_at_paragraph(&paths, pi, sections);
        if exclude.contains(&current.as_str()) {
            continue;
        }
        if let Some(p) = doc.para(StoryRef::Body, path) {
            total = total.saturating_add(para_words(p) as u32);
        }
    }
    total
}

fn section_at_paragraph(paths: &[Path], para_index: usize, sections: &[LocatedSection]) -> String {
    let mut current = sections.first().map(|s| s.id.clone()).unwrap_or_default();
    for loc in sections {
        if let Some(i) = paths.iter().position(|p| p == &loc.path)
            && i <= para_index
        {
            current = loc.id.clone();
        }
    }
    current
}

fn para_words(p: &Paragraph) -> usize {
    p.plain_text().split_whitespace().filter(|w| !w.is_empty()).count()
}

fn min_font_size(doc: &Document, check: &CheckSpec, id: &str, out: &mut Vec<Finding>) {
    let Some(min_pt) = check.pt else { return };
    for path in doc.para_paths(StoryRef::Body) {
        let Some(para) = doc.para(StoryRef::Body, &path) else { continue };
        let para_style = para.props.style.as_deref();
        let mut off = 0usize;
        for run in &para.runs {
            let chr = doc.styles.resolve_char(para_style, &run.props);
            if chr.size + 0.01 < min_pt {
                let size = chr.size;
                let start = Pos { story: StoryRef::Body, path: path.clone(), off };
                let end = Pos { story: StoryRef::Body, path: path.clone(), off: off.saturating_add(run.len) };
                let source = &check.source;
                out.push(Finding::at(id, "error", format!("Type size {size} pt is below the minimum {min_pt} pt ({source})"), start, end));
                if out.len() >= MAX_FINDINGS {
                    return;
                }
            }
            off = off.saturating_add(run.len);
        }
    }
}

fn min_margins(doc: &Document, spec: &DocSpec, check: &CheckSpec, id: &str, out: &mut Vec<Finding>) {
    let min_pt = check.pt.or_else(|| {
        let p = &spec.page;
        [p.top, p.bottom, p.left, p.right].into_iter().flatten().reduce(f32::min)
    });
    let Some(min_pt) = min_pt else { return };
    let sect = doc.sections().into_iter().next().map(|(_, s)| s);
    let Some(sect) = sect else { return };
    for (name, val) in [("top", sect.margin_top), ("bottom", sect.margin_bottom), ("left", sect.margin_left), ("right", sect.margin_right)] {
        if val + 0.01 < min_pt {
            let source = &check.source;
            out.push(Finding::error(id, format!("{name} margin {val} pt is below the minimum {min_pt} pt ({source})")));
        }
    }
}

fn min_line_spacing(doc: &Document, spec: &DocSpec, check: &CheckSpec, id: &str, out: &mut Vec<Finding>) {
    let min_mult = check.spacing.or_else(|| {
        spec.styles
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case("Normal"))
            .and_then(|s| s.para.as_ref())
            .and_then(|p| p.line_spacing)
            .and_then(line_spacing_multiple)
    });
    let Some(min_mult) = min_mult else { return };
    let except: Vec<String> = if check.except_styles.is_empty() {
        spec.styles
            .iter()
            .filter(|s| s.para.as_ref().and_then(|p| p.line_spacing).and_then(line_spacing_multiple).is_some_and(|m| m + 0.01 < min_mult))
            .map(|s| s.name.clone())
            .collect()
    } else {
        check.except_styles.clone()
    };
    for path in doc.para_paths(StoryRef::Body) {
        let Some(para) = doc.para(StoryRef::Body, &path) else { continue };
        if except.iter().any(|n| para_style_name(doc, para).is_some_and(|s| s.eq_ignore_ascii_case(n))) {
            continue;
        }
        let rp = doc.styles.resolve_para(&para.props);
        let mult = line_spacing_multiple(rp.line_spacing);
        if let Some(m) = mult
            && m + 0.01 < min_mult
        {
            let start = Pos { story: StoryRef::Body, path: path.clone(), off: 0 };
            let end = Pos { story: StoryRef::Body, path: path.clone(), off: para.text.len() };
            let source = &check.source;
            out.push(Finding::at(id, "error", format!("Line spacing {m} is below the minimum {min_mult} ({source})"), start, end));
            if out.len() >= MAX_FINDINGS {
                return;
            }
        }
    }
}

fn line_spacing_multiple(ls: LineSpacing) -> Option<f32> {
    match ls {
        LineSpacing::Multiple(v) if v.is_finite() => Some(v),
        _ => None,
    }
}

fn no_hyperlinks(doc: &Document, id: &str, out: &mut Vec<Finding>) {
    for path in doc.para_paths(StoryRef::Body) {
        let Some(para) = doc.para(StoryRef::Body, &path) else { continue };
        let para_style = para.props.style.as_deref();
        let mut off = 0usize;
        for run in &para.runs {
            let chr = doc.styles.resolve_char(para_style, &run.props);
            if chr.link.is_some() {
                let start = Pos { story: StoryRef::Body, path: path.clone(), off };
                let end = Pos { story: StoryRef::Body, path: path.clone(), off: off.saturating_add(run.len) };
                out.push(Finding::at(id, "error", "Active hyperlinks are not allowed in this filing", start, end));
                if out.len() >= MAX_FINDINGS {
                    return;
                }
            }
            off = off.saturating_add(run.len);
        }
    }
}

fn text_searchable_pdf(doc: &Document, id: &str, out: &mut Vec<Finding>) {
    let bytes = match io::save_bytes_opts("export.pdf", doc, &io::ExportOptions::default()) {
        Ok(b) => b,
        Err(e) => {
            out.push(Finding::error(id, format!("PDF export failed: {e}")));
            return;
        }
    };
    if !pdf_bytes_have_text(&bytes) {
        out.push(Finding::error(id, "PDF export is not text-searchable"));
    }
}

/// True when the PDF bytes look like they carry real text (not a pure image scan).
fn pdf_bytes_have_text(bytes: &[u8]) -> bool {
    if bytes.len() < 8 || !bytes.starts_with(b"%PDF") {
        return false;
    }
    let sample = bytes.len().min(512 * 1024);
    let head = &bytes[..sample];
    head.windows(2).any(|w| w == b"BT") || head.windows(5).any(|w| w == b"/Font")
}

/// Findings as JSON for `law.checks.run`.
pub fn findings_json(findings: &[Finding]) -> Vec<Value> {
    findings.iter().map(|f| wordcraft_engine::lint::finding_json(f, wordcraft_engine::cmd::pos_json)).collect()
}

/// Store applied profile metadata on the document and register export checks on the session.
pub fn remember_applied(session: &mut Session, profile_id: &str, doc_type: &str) {
    session.doc.set_custom_prop(PROP_PROFILE, profile_id);
    session.doc.set_custom_prop(PROP_DOC_TYPE, doc_type);
    session.set_export_check_commands(vec![("law.checks.run".into(), json!({"id": profile_id, "doc": doc_type, "forExport": true}))]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_text_sniffing() {
        assert!(!pdf_bytes_have_text(b"not a pdf"));
        assert!(pdf_bytes_have_text(b"%PDF-1.4\n1 0 obj<<>>endobj\nBT /F1 12 Tf (Hi) Tj ET"));
    }
}

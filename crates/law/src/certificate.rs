//! Certificate of compliance: profile-driven text with a live word-count field.

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Block, Document, Paragraph, Path, StoryRef};
use wordcraft_engine::{CmdError, Session};

use crate::checks::{CERT_SECTION_ID, PROP_DOC_TYPE, PROP_PROFILE, word_count, word_limit_finding};
use crate::cmd::store;
use crate::profile::{DocSpec, Profile};
use crate::store::LawError;

/// Field instruction for the live word count (updated on field rebuild and before export).
pub const FIELD_WORDCOUNT: &str = "LAWWORDCOUNT";

const SOFTWARE: &str = "WordCraft";

/// Missouri appellate brief certificate (Rule 84.06(c)); placeholders filled at insert time.
const MO_TEMPLATE: &str = "I certify that this brief contains {wordCount} words, excluding the portions of the brief exempted by Rule 84.04, does not exceed the word limit of {wordLimit} words, and complies with the type-volume requirements of Supreme Court Rule 55.03. The brief is set in {font} {fontSize}-point type and was prepared with {software}.";

/// Federal appellate brief certificate (FRAP 32(g)).
const FEDERAL_TEMPLATE: &str = "I certify that this brief complies with the type-volume limitation of the Federal Rules of Appellate Procedure and contains {wordCount} words, excluding the portions of the document required by Rule 32(f) to be excluded. The word limit for this document is {wordLimit} words. The brief is set in {font} {fontSize}-point type and was prepared with {software}.";

struct TemplateValues {
    word_count: u32,
    word_limit: u32,
    rule: String,
    font: String,
    font_size: String,
}

/// Insert or refresh the certificate of compliance for `doc_type` in `profile`.
pub fn insert_or_refresh(session: &mut Session, profile: &Profile, doc_type: &str) -> Result<Value, LawError> {
    let spec = profile.docs.get(doc_type).ok_or_else(|| missing_doc(profile, doc_type))?;
    let sec = spec
        .sections
        .iter()
        .find(|s| s.id == CERT_SECTION_ID)
        .ok_or_else(|| LawError::Failed(format!("profile `{}` / `{doc_type}` has no `{CERT_SECTION_ID}` section", profile.id)))?;
    let template = resolve_template(profile, spec)?;
    let values = template_values(&session.doc, profile, doc_type, spec, &template)?;
    session.checkpoint("Certificate of Compliance");
    write_certificate(session, spec, sec, &template, &values)?;
    refresh_wordcount_fields(session).map_err(|e| LawError::Failed(e.to_string()))?;
    let mut out = json!({
        "profile": profile.id,
        "doc": doc_type,
        "wordCount": values.word_count,
        "wordLimit": values.word_limit,
        "section": CERT_SECTION_ID,
    });
    if let Some(f) = word_limit_finding(&session.doc, profile, doc_type)? {
        out["findings"] = json!(crate::checks::findings_json(&[f]));
    }
    Ok(out)
}

/// Refresh every [`FIELD_WORDCOUNT`] result in the document (export hook and `law.certificate.refresh`).
pub fn refresh_wordcount_fields(session: &mut Session) -> Result<(), CmdError> {
    let profile_id = session
        .doc
        .custom_prop(PROP_PROFILE)
        .ok_or_else(|| CmdError::Params("no court profile is applied to this document (apply a profile first)".into()))?;
    let doc_type = session.doc.custom_prop(PROP_DOC_TYPE).ok_or_else(|| CmdError::Params("no court document type is set on this document".into()))?;
    let profile = store().get(profile_id).cloned().ok_or_else(|| CmdError::Failed(format!("no court profile `{profile_id}`")))?;
    let count = word_count(&session.doc, &profile, doc_type).map_err(|e| CmdError::Failed(e.to_string()))?;
    let want = field_name_instr(FIELD_WORDCOUNT);
    let mut touched = false;
    for story in session.doc.counted_stories() {
        for path in session.doc.para_paths(story) {
            let Ok(para) = session.doc.para_mut(story, &path) else { continue };
            let mut changed = false;
            for obj in &mut para.objects {
                if let InlineObject::Field { instr, result, locked: false, .. } = obj
                    && field_name_instr(instr.as_str()) == want
                {
                    let text = count.to_string();
                    if *result != text {
                        *result = text;
                        changed = true;
                    }
                }
            }
            if changed {
                para.touch();
                touched = true;
            }
        }
    }
    if touched {
        session.touch();
    }
    Ok(())
}

/// Hook for [`wordcraft_engine::hooks::register_on_update_fields`].
pub fn on_update_fields(session: &mut Session) -> Result<(), CmdError> {
    if session.doc.custom_prop(PROP_PROFILE).is_some() && session.doc.custom_prop(PROP_DOC_TYPE).is_some() {
        refresh_wordcount_fields(session)?;
    }
    Ok(())
}

fn missing_doc(profile: &Profile, doc_type: &str) -> LawError {
    LawError::Failed(format!("profile `{}` has no document type `{doc_type}` (it has {})", profile.id, profile.doc_types().join(", ")))
}

fn resolve_template(profile: &Profile, spec: &DocSpec) -> Result<String, LawError> {
    if let Some(c) = &spec.certificate_of_compliance {
        return Ok(c.template.clone());
    }
    match profile.id.as_str() {
        "missouri-appellate-brief" => Ok(MO_TEMPLATE.to_string()),
        "federal-appellate-brief" => Ok(FEDERAL_TEMPLATE.to_string()),
        _ => Err(LawError::Failed(format!(
            "profile `{}` has no certificateOfCompliance template for this document type (add one to the profile file)",
            profile.id
        ))),
    }
}

fn template_values(doc: &Document, profile: &Profile, doc_type: &str, spec: &DocSpec, template: &str) -> Result<TemplateValues, LawError> {
    let word_count = word_count(doc, profile, doc_type)?;
    let wl = spec
        .checks
        .iter()
        .find(|c| c.kind == "wordLimit")
        .ok_or_else(|| LawError::Failed(format!("profile `{}` / `{doc_type}` has no wordLimit check", profile.id)))?;
    let word_limit = wl.limit.ok_or_else(|| LawError::Failed(format!("profile `{}` wordLimit has no limit", profile.id)))?;
    let rule = cert_rule(spec, wl);
    let (font, font_size) = typeface(spec);
    let _ = template;
    Ok(TemplateValues { word_count, word_limit, rule, font, font_size })
}

fn cert_rule(spec: &DocSpec, wl: &crate::profile::CheckSpec) -> String {
    spec.certificate_of_compliance
        .as_ref()
        .and_then(|c| (!c.source.is_empty()).then(|| c.source.clone()))
        .or_else(|| (!wl.source.is_empty()).then(|| wl.source.clone()))
        .unwrap_or_else(|| spec.source.clone())
}

fn typeface(spec: &DocSpec) -> (String, String) {
    if let Some(c) = spec.checks.iter().find(|x| x.kind == "minFontSize") {
        let font = c.font.clone().or_else(|| {
            spec.styles.iter().find(|s| s.name.eq_ignore_ascii_case("Normal")).and_then(|s| s.chr.as_ref()).and_then(|chr| chr.font.clone())
        });
        if let (Some(font), Some(pt)) = (font, c.pt) {
            let size = if pt.fract().abs() < 0.01 { format!("{:.0}", pt) } else { format!("{pt}") };
            return (font, size);
        }
    }
    if let Some(st) = spec.styles.iter().find(|s| s.name.eq_ignore_ascii_case("Normal"))
        && let Some(chr) = &st.chr
    {
        let font = chr.font.clone().unwrap_or_else(|| "Times New Roman".into());
        let pt = chr.size.unwrap_or(12.0);
        let size = if pt.fract().abs() < 0.01 { format!("{:.0}", pt) } else { format!("{pt}") };
        return (font, size);
    }
    ("Times New Roman".into(), "12".into())
}

fn write_certificate(
    session: &mut Session,
    spec: &DocSpec,
    sec: &crate::profile::SectionSpec,
    template: &str,
    values: &TemplateValues,
) -> Result<(), LawError> {
    let heading = sec.name.trim();
    let body_paths = session.doc.para_paths(StoryRef::Body);
    let heading_i = find_heading_index(&session.doc, &body_paths, heading);
    let next_i = next_section_index(&session.doc, spec, &body_paths, heading_i);
    let cert_style = sec.style.as_deref().or(Some("Certificate"));
    let style_id = cert_style.and_then(|name| session.doc.styles.find(name).map(|s| s.id.clone()));

    let Some(hi) = heading_i else {
        append_certificate(session, heading, style_id.as_deref(), template, values)?;
        return Ok(());
    };
    if hi + 1 < next_i {
        for path in body_paths[hi + 1..next_i].iter().rev() {
            session.doc.remove_block(StoryRef::Body, path).map_err(|e| LawError::Failed(e.to_string()))?;
        }
    }
    for (insert_at, para) in (hi + 1..).zip(paragraphs_from_template(template, values, style_id.as_deref())) {
        session.doc.insert_block(StoryRef::Body, &Path::top(insert_at), Block::Para(para)).map_err(|e| LawError::Failed(e.to_string()))?;
    }
    session.touch();
    Ok(())
}

fn append_certificate(session: &mut Session, heading: &str, style_id: Option<&str>, template: &str, values: &TemplateValues) -> Result<(), LawError> {
    let at = session.doc.para_paths(StoryRef::Body).len();
    let mut head = Paragraph::with_text(heading, CharProps::default());
    if let Some(id) = style_id {
        head.props.style = Some(id.to_string());
    }
    session.doc.insert_block(StoryRef::Body, &Path::top(at), Block::Para(head)).map_err(|e| LawError::Failed(e.to_string()))?;
    for (insert_at, para) in (at + 1..).zip(paragraphs_from_template(template, values, style_id)) {
        session.doc.insert_block(StoryRef::Body, &Path::top(insert_at), Block::Para(para)).map_err(|e| LawError::Failed(e.to_string()))?;
    }
    session.touch();
    Ok(())
}

fn find_heading_index(doc: &Document, paths: &[Path], heading: &str) -> Option<usize> {
    paths.iter().position(|path| doc.para(StoryRef::Body, path).is_some_and(|p| p.plain_text().trim().eq_ignore_ascii_case(heading)))
}

fn next_section_index(doc: &Document, spec: &DocSpec, paths: &[Path], heading_i: Option<usize>) -> usize {
    let Some(hi) = heading_i else { return paths.len() };
    let cert_pos = spec.sections.iter().position(|s| s.id == CERT_SECTION_ID).unwrap_or(hi);
    let after: Vec<&str> = spec.sections.iter().skip(cert_pos.saturating_add(1)).map(|s| s.name.trim()).collect();
    for (offset, path) in paths.iter().enumerate().skip(hi + 1) {
        let Some(para) = doc.para(StoryRef::Body, path) else { continue };
        let plain = para.plain_text();
        let text = plain.trim();
        if after.iter().any(|name| text.eq_ignore_ascii_case(name)) {
            return offset;
        }
    }
    paths.len()
}

fn paragraphs_from_template(template: &str, values: &TemplateValues, style_id: Option<&str>) -> Vec<Paragraph> {
    template.split('\n').map(|line| build_paragraph(substitute_line(line, values), values.word_count, style_id)).collect()
}

fn field_name_instr(instr: &str) -> String {
    instr.split_whitespace().next().unwrap_or("").trim_start_matches('=').to_ascii_uppercase()
}

fn substitute_line(line: &str, values: &TemplateValues) -> String {
    let mut s = line.to_string();
    for (from, to) in [
        ("{wordLimit}", values.word_limit.to_string()),
        ("{rule}", values.rule.clone()),
        ("{font}", values.font.clone()),
        ("{fontSize}", values.font_size.clone()),
        ("{software}", SOFTWARE.to_string()),
    ] {
        s = s.replace(from, &to);
    }
    s
}

fn build_paragraph(text: String, word_count: u32, style_id: Option<&str>) -> Paragraph {
    let mut para = Paragraph::new();
    if let Some(id) = style_id {
        para.props.style = Some(id.to_string());
    }
    let props = CharProps::default();
    let marker = "{wordCount}";
    let mut off = 0usize;
    let mut rest = text.as_str();
    while let Some(i) = rest.find(marker) {
        let before = &rest[..i];
        if !before.is_empty() {
            let _ = para.insert_text(off, before, &props);
            off = off.saturating_add(before.len());
        }
        let result = word_count.to_string();
        let _ = para.insert_object(off, InlineObject::Field { instr: FIELD_WORDCOUNT.into(), result, locked: false, code: false }, &props);
        off = off.saturating_add(OBJ.len_utf8());
        rest = &rest[i + marker.len()..];
    }
    if !rest.is_empty() {
        let _ = para.insert_text(off, rest, &props);
    }
    para
}

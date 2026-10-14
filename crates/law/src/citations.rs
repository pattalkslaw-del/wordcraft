//! Citation finder and form checker (suggestions only; never claims an authority exists).

use wordcraft_doc::{Document, Pos};
use wordcraft_engine::lint::{Finding, MAX_FINDINGS};

use crate::citation_pack::CompiledPack;

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
                    let slice = m.as_str();
                    if rule.valid.as_ref().is_some_and(|v| v.is_match(slice)) {
                        continue;
                    }
                    let start = Pos { story, path: path.clone(), off: m.start() };
                    let end = Pos { story, path: path.clone(), off: m.end() };
                    let msg = format!("{} ({})", rule.message, rule.source);
                    out.push(Finding::at(rule.id.clone(), "warning", msg, start, end));
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

//! Legal spell-check supplement and confusion-pair proofing for LawCraft.

use std::sync::OnceLock;

use serde_json::{Value, json};
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_engine::{Session, lint::Finding};

use crate::checks::findings_json;
use wordcraft_proof::{Issue, IssueKind, ProofLang};

const LEGAL_WORDS_TXT: &str = include_str!("../data/legal-words.txt");

/// Every legal base form from [`LEGAL_WORDS_TXT`], sorted.
pub fn legal_words() -> &'static [&'static str] {
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    WORDS
        .get_or_init(|| {
            let mut v: Vec<&'static str> = LEGAL_WORDS_TXT.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
            v.sort();
            v.dedup();
            v
        })
        .as_slice()
}

/// Hook LawCraft into WordCraft proofing (legal dictionary + confusion checks).
pub fn install() {
    wordcraft_proof::set_module_en_spelling(Some(legal_words()));
    wordcraft_proof::set_module_en_grammar(Some(confusion_issues));
}

/// Drop LawCraft proofing hooks (module off or before a fresh [`crate::install`]).
pub(crate) fn uninstall() {
    wordcraft_proof::set_module_en_spelling(None);
    wordcraft_proof::set_module_en_grammar(None);
}

/// Confusion issues in one paragraph of English text (for proofing and [`confusion_findings`]).
pub fn confusion_issues(text: &str) -> Vec<Issue> {
    let mut out = Vec::new();
    for (a, b) in wordcraft_proof::words(text) {
        let w = text.get(a..b).unwrap_or("");
        let lower = w.to_lowercase();
        let following = following_words(text, b, 6);
        let preceding = preceding_words(text, a, 4);
        for rule in CONFUSION_RULES {
            if !lower.eq_ignore_ascii_case(rule.wrong) {
                continue;
            }
            let context_ok = (rule.following.is_empty() || words_contain_sequence(&following, rule.following))
                && (rule.preceding.is_empty() || words_contain_sequence(&preceding, rule.preceding));
            if context_ok {
                out.push(Issue {
                    start: a,
                    end: b,
                    kind: IssueKind::Grammar,
                    message: rule.message.into(),
                    suggestions: vec![capitalize_like(w, rule.right)],
                    lang: ProofLang::En,
                });
            }
        }
    }
    out
}

/// Scan the open document's body for confusion-pair issues.
pub fn confusion_findings(doc: &Document) -> Vec<Finding> {
    let mut out = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(para) = doc.para(StoryRef::Body, &path) else { continue };
        let text = para.plain_text();
        if text.trim().is_empty() || text.len() > 100_000 {
            continue;
        }
        for issue in confusion_issues(&text) {
            let start = Pos { story: StoryRef::Body, path: path.clone(), off: issue.start };
            let end = Pos { story: StoryRef::Body, path: path.clone(), off: issue.end };
            out.push(Finding::at("law.confusion", "suggestion", issue.message.clone(), start, end));
            if out.len() >= wordcraft_engine::lint::MAX_FINDINGS {
                return out;
            }
        }
    }
    out
}

/// Run confusion checks and optionally store them for Review › Proofing.
pub fn run_confusion_checks(session: &mut Session, store: bool) -> Value {
    let findings = confusion_findings(&session.doc);
    let items = findings_json(&findings);
    if store {
        session.set_lint_findings(findings);
    }
    json!({"findings": items})
}

struct ConfusionRule {
    wrong: &'static str,
    right: &'static str,
    message: &'static str,
    /// When non-empty, every token must appear in order in the words after `wrong`.
    following: &'static [&'static str],
    /// When non-empty, every token must appear in order in the words before `wrong`.
    preceding: &'static [&'static str],
}

/// Context-sensitive legal word-pair checks (original wording; no manual copied).
const CONFUSION_RULES: &[ConfusionRule] = &[
    ConfusionRule {
        wrong: "statue",
        right: "statute",
        message: "\"Statute\" is the law; \"statue\" is a sculpture — did you mean statute of limitations?",
        following: &["of", "limitations"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "principle",
        right: "principal",
        message: "In legal writing, the main claim or amount is usually \"principal\", not \"principle\".",
        following: &["issue"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "principle",
        right: "principal",
        message: "The primary relief sought is the \"principal\" claim, not \"principle\".",
        following: &["claim"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "principal",
        right: "principle",
        message: "A rule of law is a \"principle\", not \"principal\".",
        following: &["of", "law"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "council",
        right: "counsel",
        message: "A party's lawyer is \"counsel\", not \"council\" (a governing body).",
        following: &["for", "the"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "council",
        right: "counsel",
        message: "Court-appointed representation is \"counsel\", not \"council\".",
        following: &["of", "record"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "affect",
        right: "effect",
        message: "A result or outcome is an \"effect\"; \"affect\" is usually the verb.",
        following: &["of", "the"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "effect",
        right: "affect",
        message: "To influence a ruling is to \"affect\" it, not \"effect\" (except \"effect change\").",
        following: &["the", "outcome"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "precedence",
        right: "precedent",
        message: "A prior appellate decision is \"precedent\", not \"precedence\" (priority).",
        following: &[],
        preceding: &["binding"],
    },
    ConfusionRule {
        wrong: "precedence",
        right: "precedent",
        message: "Controlling case law is \"precedent\", not \"precedence\".",
        following: &[],
        preceding: &["controlling"],
    },
    ConfusionRule {
        wrong: "proscribe",
        right: "prescribe",
        message: "A doctor \"prescribes\" treatment; \"proscribe\" means to forbid.",
        following: &["medication"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "prescribe",
        right: "proscribe",
        message: "To forbid conduct by rule is to \"proscribe\" it, not \"prescribe\".",
        following: &["conduct"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "averse",
        right: "adverse",
        message: "An opposing party or interest is \"adverse\", not \"averse\" (unwilling).",
        following: &["possession"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "adverse",
        right: "averse",
        message: "Being unwilling is \"averse\"; \"adverse\" describes harm or opposition.",
        following: &["to", "risk"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "illicit",
        right: "elicit",
        message: "To draw out testimony is to \"elicit\" it, not \"illicit\" (unlawful).",
        following: &["testimony"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "illicit",
        right: "elicit",
        message: "To draw out a response is to \"elicit\" it, not \"illicit\".",
        following: &["a", "response"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "compliment",
        right: "complement",
        message: "To complete or supplement the record is to \"complement\" it, not \"compliment\".",
        following: &["the", "record"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "complement",
        right: "compliment",
        message: "To praise is to \"compliment\", not \"complement\" (complete).",
        following: &["the", "court"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "discreet",
        right: "discrete",
        message: "Separate or distinct issues are \"discrete\", not \"discreet\" (tactful).",
        following: &["issues"],
        preceding: &[],
    },
    ConfusionRule {
        wrong: "discrete",
        right: "discreet",
        message: "Careful or tactful conduct is \"discreet\", not \"discrete\" (separate).",
        following: &["investigation"],
        preceding: &[],
    },
];

fn preceding_words(text: &str, before: usize, max: usize) -> Vec<String> {
    let head = text.get(..before).unwrap_or("");
    let mut words: Vec<String> = wordcraft_proof::words(head).into_iter().filter_map(|(a, b)| head.get(a..b).map(|w| w.to_lowercase())).collect();
    if words.len() > max {
        words = words.split_off(words.len() - max);
    }
    words
}

fn following_words(text: &str, from: usize, max: usize) -> Vec<String> {
    let tail = text.get(from..).unwrap_or("");
    wordcraft_proof::words(tail).into_iter().take(max).filter_map(|(a, b)| tail.get(a..b).map(|w| w.to_lowercase())).collect()
}

fn words_contain_sequence(haystack: &[String], needle: &[&str]) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut i = 0;
    for w in haystack {
        if w == needle[i] {
            i += 1;
            if i == needle.len() {
                return true;
            }
        }
    }
    false
}

fn capitalize_like(original: &str, replacement: &str) -> String {
    if original.chars().next().is_some_and(|c| c.is_uppercase()) {
        let mut s = replacement.to_string();
        if let Some(c) = s.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        s
    } else {
        replacement.to_string()
    }
}

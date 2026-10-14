//! Legal spell-check supplement and confusion-pair proofing for LawCraft.

use serde_json::{Value, json};
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_engine::{Session, lint::Finding};
use wordcraft_proof::{Issue, IssueKind, ModuleEn, ProofLang};

use crate::checks::findings_json;

const LEGAL_WORDS_TXT: &str = include_str!("../data/legal-words.txt");

/// Every legal base form from [`LEGAL_WORDS_TXT`], sorted.
pub fn legal_words() -> &'static [&'static str] {
    static WORDS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    WORDS
        .get_or_init(|| {
            let mut v: Vec<&'static str> = LEGAL_WORDS_TXT.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
            v.sort();
            v.dedup();
            v
        })
        .as_slice()
}

/// Per-session English proofing hooks for LawCraft (legal dictionary + confusion checks).
pub fn module_hooks() -> ModuleEn {
    ModuleEn::with_spelling(Some(legal_words())).with_grammar(Some(confusion_issues))
}

/// Confusion issues in one paragraph of English text (for proofing and [`confusion_findings`]).
pub fn confusion_issues(text: &str) -> Vec<Issue> {
    let tokens: Vec<(usize, usize, String)> =
        wordcraft_proof::words(text).into_iter().filter_map(|(a, b)| text.get(a..b).map(|w| (a, b, w.to_lowercase()))).collect();
    let mut out = Vec::new();
    for (idx, (a, b, lower)) in tokens.iter().enumerate() {
        let surface = text.get(*a..*b).unwrap_or("");
        for rule in CONFUSION_RULES {
            if !lower.eq_ignore_ascii_case(rule.wrong) {
                continue;
            }
            if !phrase_follows(&tokens, idx, text, rule.following)
                || !phrase_precedes(&tokens, idx, text, rule.preceding)
                || !any_precedes(&tokens, idx, text, rule.preceding_any)
            {
                continue;
            }
            out.push(Issue {
                start: *a,
                end: *b,
                kind: IssueKind::Grammar,
                message: rule.message.into(),
                suggestions: vec![capitalize_like(surface, rule.right)],
                lang: ProofLang::En,
            });
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
    /// When non-empty, the words immediately after `wrong` in the same sentence.
    following: &'static [&'static str],
    /// When non-empty, the exact words immediately before `wrong` in the same sentence, in order.
    preceding: &'static [&'static str],
    /// When non-empty, any one of these words may sit immediately before `wrong`. Use for a
    /// category (an auxiliary or `to` that only leads a verb) where the exact word varies but the
    /// context it signals is the same.
    preceding_any: &'static [&'static str],
}

/// Auxiliaries that only precede a *base verb*. A word after one of these is doing verb work, so
/// `illicit` there is almost certainly the confused verb `elicit`; without one, `illicit
/// testimony` ("unlawful testimony") is correct and must not fire. The infinitive/preposition `to`
/// is deliberately absent: "objection to illicit testimony" is correct, and telling an infinitival
/// `to` from a prepositional one needs part-of-speech context this checker doesn't have, so it is
/// safer to drop `to` than to risk flagging correct prose.
const VERB_LEADS: &[&str] = &["will", "would", "can", "could", "may", "might", "must", "shall", "should", "did", "does", "do"];

/// Context-sensitive legal word-pair checks (original wording; no manual copied).
const CONFUSION_RULES: &[ConfusionRule] = &[
    ConfusionRule {
        wrong: "statue",
        right: "statute",
        message: "\"Statute\" is the law; \"statue\" is a sculpture — did you mean statute of limitations?",
        following: &["of", "limitations"],
        preceding: &[],
        preceding_any: &[],
    },
    ConfusionRule {
        wrong: "principle",
        right: "principal",
        message: "In legal writing, the main claim or amount is usually \"principal\", not \"principle\".",
        following: &["issue"],
        preceding: &[],
        preceding_any: &[],
    },
    ConfusionRule {
        wrong: "principle",
        right: "principal",
        message: "The primary relief sought is the \"principal\" claim, not \"principle\".",
        following: &["claim"],
        preceding: &[],
        preceding_any: &[],
    },
    ConfusionRule {
        wrong: "principal",
        right: "principle",
        message: "A rule of law is a \"principle\", not \"principal\".",
        following: &["of", "law"],
        preceding: &[],
        preceding_any: &[],
    },
    // Note: `council for the …` is deliberately absent. "The city council for the district" is
    // correct, and immediate adjacency to `for the` doesn't make `council` likely wrong. Only the
    // unambiguous `council of record` fires below.
    ConfusionRule {
        wrong: "council",
        right: "counsel",
        message: "Court-appointed representation is \"counsel\", not \"council\" (a governing body).",
        following: &["of", "record"],
        preceding: &[],
        preceding_any: &[],
    },
    ConfusionRule {
        wrong: "affect",
        right: "effect",
        message: "A result or outcome is an \"effect\"; \"affect\" is usually the verb.",
        following: &["of", "the"],
        preceding: &[],
        preceding_any: &[],
    },
    ConfusionRule {
        wrong: "effect",
        right: "affect",
        message: "To influence a ruling is to \"affect\" it, not \"effect\" (except \"effect change\").",
        following: &["the", "outcome"],
        preceding: &[],
        preceding_any: &[],
    },
    ConfusionRule {
        wrong: "precedence",
        right: "precedent",
        message: "A prior appellate decision is \"precedent\", not \"precedence\" (priority).",
        following: &[],
        preceding: &["binding"],
        preceding_any: &[],
    },
    ConfusionRule {
        wrong: "precedence",
        right: "precedent",
        message: "Controlling case law is \"precedent\", not \"precedence\".",
        following: &[],
        preceding: &["controlling"],
        preceding_any: &[],
    },
    ConfusionRule {
        wrong: "proscribe",
        right: "prescribe",
        message: "A doctor \"prescribes\" treatment; \"proscribe\" means to forbid.",
        following: &["medication"],
        preceding: &[],
        preceding_any: &[],
    },
    // Note: `prescribe such conduct` is deliberately absent. A rule can affirmatively *prescribe*
    // conduct, so `prescribe` there is correct, not a reversed `proscribe`; adjacency to `such
    // conduct` doesn't establish that the word is likely wrong. The clear `proscribe medication`
    // case still fires below.
    ConfusionRule {
        wrong: "averse",
        right: "adverse",
        message: "An opposing party or interest is \"adverse\", not \"averse\" (unwilling).",
        following: &["possession"],
        preceding: &[],
        preceding_any: &[],
    },
    ConfusionRule {
        wrong: "adverse",
        right: "averse",
        message: "Being unwilling is \"averse\"; \"adverse\" describes harm or opposition.",
        following: &["to", "risk"],
        preceding: &[],
        preceding_any: &[],
    },
    ConfusionRule {
        wrong: "illicit",
        right: "elicit",
        message: "To draw out testimony is to \"elicit\" it, not \"illicit\" (unlawful).",
        following: &["testimony"],
        preceding: &[],
        preceding_any: VERB_LEADS,
    },
    ConfusionRule {
        wrong: "illicit",
        right: "elicit",
        message: "To draw out a response is to \"elicit\" it, not \"illicit\".",
        following: &["a", "response"],
        preceding: &[],
        preceding_any: VERB_LEADS,
    },
    ConfusionRule {
        wrong: "compliment",
        right: "complement",
        message: "To complete or supplement the record is to \"complement\" it, not \"compliment\".",
        following: &["the", "record"],
        preceding: &[],
        preceding_any: &[],
    },
    // Note: the reverse `complement the court` rule is deliberately absent. "The new tribunal will
    // complement the court" is correct — `complement` legitimately takes a court as its object — so
    // adjacency to `the court` doesn't make the word likely wrong. Only the clear
    // `compliment the record` case above fires.
    ConfusionRule {
        wrong: "discreet",
        right: "discrete",
        message: "Separate or distinct issues are \"discrete\", not \"discreet\" (tactful).",
        following: &["issues"],
        preceding: &[],
        preceding_any: &[],
    },
    ConfusionRule {
        wrong: "discrete",
        right: "discreet",
        message: "Careful or tactful conduct is \"discreet\", not \"discrete\" (separate).",
        following: &["investigation"],
        preceding: &[],
        preceding_any: &[],
    },
];

fn is_sentence_end(c: char) -> bool {
    matches!(c, '.' | '!' | '?')
}

/// Whether the raw text between two adjacent tokens holds a sentence end. Phrase words are
/// immediate neighbours, so checking each gap keeps the work proportional to the phrase length
/// instead of collecting every word in the sentence for every candidate token (which was
/// quadratic on a long sentence with many candidates).
fn gap_has_sentence_end(text: &str, from: usize, to: usize) -> bool {
    text.get(from..to).is_some_and(|gap| gap.chars().any(is_sentence_end))
}

fn phrase_follows(tokens: &[(usize, usize, String)], idx: usize, text: &str, phrase: &[&str]) -> bool {
    if phrase.is_empty() {
        return true;
    }
    let Some(first) = tokens.get(idx) else { return false };
    let mut prev_end = first.1;
    for (n, want) in phrase.iter().enumerate() {
        let Some(t) = tokens.get(idx.saturating_add(1).saturating_add(n)) else { return false };
        if t.2 != *want || gap_has_sentence_end(text, prev_end, t.0) {
            return false;
        }
        prev_end = t.1;
    }
    true
}

fn phrase_precedes(tokens: &[(usize, usize, String)], idx: usize, text: &str, phrase: &[&str]) -> bool {
    if phrase.is_empty() {
        return true;
    }
    let Some(first) = tokens.get(idx) else { return false };
    let mut next_start = first.0;
    for (n, want) in phrase.iter().rev().enumerate() {
        let Some(i) = idx.checked_sub(n.saturating_add(1)) else { return false };
        let Some(t) = tokens.get(i) else { return false };
        if t.2 != *want || gap_has_sentence_end(text, t.1, next_start) {
            return false;
        }
        next_start = t.0;
    }
    true
}

/// Whether any word in `set` sits immediately before the token at `idx`, with no sentence end
/// between. A set (unlike [`phrase_precedes`]'s ordered phrase) matches any one member, so rules
/// can name a category such as the auxiliaries and `to` that only lead a verb.
fn any_precedes(tokens: &[(usize, usize, String)], idx: usize, text: &str, set: &[&str]) -> bool {
    if set.is_empty() {
        return true;
    }
    let Some(i) = idx.checked_sub(1) else { return false };
    let (Some(prev), Some(cur)) = (tokens.get(i), tokens.get(idx)) else { return false };
    set.contains(&prev.2.as_str()) && !gap_has_sentence_end(text, prev.1, cur.0)
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

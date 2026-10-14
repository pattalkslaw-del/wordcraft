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
    for start_idx in 0..tokens.len() {
        for rule in CONFUSION_PHRASES {
            let need = rule.words.len();
            if start_idx + need > tokens.len() {
                continue;
            }
            if !rule.words.iter().enumerate().all(|(n, want)| word_matches(want, &tokens[start_idx + n].2)) {
                continue;
            }
            if !(0..need - 1).all(|n| gap_allows_phrase_join(text, tokens[start_idx + n].1, tokens[start_idx + n + 1].0)) {
                continue;
            }
            let wrong = start_idx + rule.wrong_index;
            let (a, b, _) = tokens[wrong];
            let surface = text.get(a..b).unwrap_or("");
            let wrong_base = rule.words[rule.wrong_index];
            out.push(Issue {
                start: a,
                end: b,
                kind: IssueKind::Grammar,
                message: rule.message.into(),
                suggestions: vec![inflected_replacement(surface, wrong_base, rule.right)],
                lang: ProofLang::En,
            });
        }
    }
    out.sort_by_key(|i| i.start);
    out.dedup_by_key(|i| i.start);
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

struct ConfusionPhrase {
    /// Lowercase words in order (matched case-insensitively with plural/possessive variants).
    words: &'static [&'static str],
    /// Index into [`Self::words`] of the misspelled/confused token to highlight.
    wrong_index: usize,
    right: &'static str,
    message: &'static str,
}

/// Fixed wrong phrases only — no single-token guesses or lookahead (original wording; no manual copied).
///
/// Every entry must be unambiguous: it fires only when the highlighted word cannot be right. Where
/// no exact phrase is unambiguous the pair is omitted rather than guessed at — for example
/// `prescribe`/`proscribe`, because a subject that can forbid can also legitimately `proscribe`
/// medication ("Doctors proscribe medication that is contraindicated"); `controlling precedence`,
/// because `precedence` correctly means priority; and `complement`/`compliment`, because a "full
/// compliment of the judge" is correct praise, so no `compliment` phrase fires.
const CONFUSION_PHRASES: &[ConfusionPhrase] = &[
    ConfusionPhrase {
        words: &["statue", "of", "limitations"],
        wrong_index: 0,
        right: "statute",
        message: "\"Statute\" is the law; \"statue\" is a sculpture — did you mean statute of limitations?",
    },
    ConfusionPhrase {
        words: &["statue", "of", "frauds"],
        wrong_index: 0,
        right: "statute",
        message: "\"Statute\" is the law; \"statue\" is a sculpture — did you mean statute of frauds?",
    },
    ConfusionPhrase {
        words: &["principle", "issue"],
        wrong_index: 0,
        right: "principal",
        message: "In legal writing, the main claim or amount is usually \"principal\", not \"principle\".",
    },
    ConfusionPhrase {
        words: &["principle", "claim"],
        wrong_index: 0,
        right: "principal",
        message: "The primary relief sought is the \"principal\" claim, not \"principle\".",
    },
    ConfusionPhrase {
        words: &["principle", "amount"],
        wrong_index: 0,
        right: "principal",
        message: "The sum at stake is the \"principal\" amount, not \"principle\".",
    },
    ConfusionPhrase {
        words: &["principle", "place", "of", "business"],
        wrong_index: 0,
        right: "principal",
        message: "The main place of business is the \"principal\" place, not \"principle\".",
    },
    ConfusionPhrase {
        words: &["principal", "of", "law"],
        wrong_index: 0,
        right: "principle",
        message: "A rule of law is a \"principle\", not \"principal\".",
    },
    ConfusionPhrase {
        words: &["council", "for", "the", "plaintiff"],
        wrong_index: 0,
        right: "counsel",
        message: "Court-appointed representation is \"counsel\", not \"council\" (a governing body).",
    },
    ConfusionPhrase {
        words: &["council", "for", "the", "defendant"],
        wrong_index: 0,
        right: "counsel",
        message: "Court-appointed representation is \"counsel\", not \"council\" (a governing body).",
    },
    ConfusionPhrase {
        words: &["council", "for", "the", "appellant"],
        wrong_index: 0,
        right: "counsel",
        message: "Court-appointed representation is \"counsel\", not \"council\" (a governing body).",
    },
    ConfusionPhrase {
        words: &["council", "for", "the", "respondent"],
        wrong_index: 0,
        right: "counsel",
        message: "Court-appointed representation is \"counsel\", not \"council\" (a governing body).",
    },
    ConfusionPhrase {
        words: &["council", "of", "record"],
        wrong_index: 0,
        right: "counsel",
        message: "Court-appointed representation is \"counsel\", not \"council\" (a governing body).",
    },
    ConfusionPhrase {
        words: &["opposing", "council"],
        wrong_index: 1,
        right: "counsel",
        message: "The other side's lawyer is \"counsel\", not \"council\".",
    },
    ConfusionPhrase {
        words: &["the", "affect", "of"],
        wrong_index: 1,
        right: "effect",
        message: "A result or outcome is an \"effect\"; \"affect\" is usually the verb.",
    },
    ConfusionPhrase {
        words: &["affect", "a", "change"],
        wrong_index: 0,
        right: "effect",
        message: "To bring about change is usually to \"effect\" it, not \"affect\".",
    },
    ConfusionPhrase {
        words: &["binding", "precedence"],
        wrong_index: 1,
        right: "precedent",
        message: "A prior appellate decision is \"precedent\", not \"precedence\" (priority).",
    },
    ConfusionPhrase {
        words: &["averse", "possession"],
        wrong_index: 0,
        right: "adverse",
        message: "An opposing party or interest is \"adverse\", not \"averse\" (unwilling).",
    },
    ConfusionPhrase {
        words: &["adverse", "to", "risk"],
        wrong_index: 0,
        right: "averse",
        message: "Being unwilling is \"averse\"; \"adverse\" describes harm or opposition.",
    },
    ConfusionPhrase {
        words: &["adverse", "to", "doing"],
        wrong_index: 0,
        right: "averse",
        message: "Being unwilling is \"averse\"; \"adverse\" describes harm or opposition.",
    },
    ConfusionPhrase {
        words: &["illicit", "testimony"],
        wrong_index: 0,
        right: "elicit",
        message: "To draw out testimony is to \"elicit\" it, not \"illicit\" (unlawful).",
    },
    ConfusionPhrase {
        words: &["illicit", "a", "response"],
        wrong_index: 0,
        right: "elicit",
        message: "To draw out a response is to \"elicit\" it, not \"illicit\".",
    },
    ConfusionPhrase {
        words: &["could", "illicit", "a", "response"],
        wrong_index: 1,
        right: "elicit",
        message: "To draw out a response is to \"elicit\" it, not \"illicit\".",
    },
    ConfusionPhrase {
        words: &["discreet", "issues"],
        wrong_index: 0,
        right: "discrete",
        message: "Separate or distinct issues are \"discrete\", not \"discreet\" (tactful).",
    },
];

/// Only plain whitespace may join the words of an exact confusion phrase. Any punctuation
/// (periods, semicolons, colons, commas, parentheses, quotes, line breaks) ends the phrase, so
/// unrelated clauses can never be stitched into a false positive.
fn gap_allows_phrase_join(text: &str, end_a: usize, start_b: usize) -> bool {
    let Some(gap) = text.get(end_a..start_b) else {
        return false;
    };
    !gap.is_empty() && gap.chars().all(|c| c.is_whitespace() && !matches!(c, '\n' | '\r'))
}

/// Whether token `actual` matches the expected dictionary word (case-insensitive, plural/possessive).
fn word_matches(expected: &str, actual: &str) -> bool {
    if expected.eq_ignore_ascii_case(actual) {
        return true;
    }
    let actual = actual.to_lowercase();
    let expected = expected.to_lowercase();
    if actual == expected {
        return true;
    }
    if actual == format!("{expected}'s") || actual == format!("{expected}’s") {
        return true;
    }
    if actual == format!("{expected}s") || actual == format!("{expected}es") {
        return true;
    }
    if let Some(stem) = actual.strip_suffix('s')
        && (stem == expected || format!("{stem}e") == expected)
    {
        return true;
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

/// Build a suggestion that keeps plural/possessive endings from the matched wrong word.
fn inflected_replacement(surface: &str, wrong_base: &str, right_base: &str) -> String {
    let suffix = inflection_suffix(surface, wrong_base);
    let combined = match suffix {
        Some(suf) => format!("{right_base}{suf}"),
        None => right_base.to_string(),
    };
    capitalize_like(surface, &combined)
}

/// Suffix on `surface` after the inflected form of `base` (e.g. `statues` → `"s"`).
fn inflection_suffix(surface: &str, base: &str) -> Option<String> {
    let lower = surface.to_lowercase();
    let base_lower = base.to_lowercase();
    if lower == base_lower {
        return Some(String::new());
    }
    for apost in ["'s", "’s"] {
        if lower == format!("{base_lower}{apost}") {
            return Some(apost.to_string());
        }
    }
    if lower == format!("{base_lower}s") {
        return Some("s".to_string());
    }
    if lower == format!("{base_lower}es") {
        return Some("es".to_string());
    }
    if let Some(stem) = lower.strip_suffix('s')
        && (stem == base_lower || format!("{stem}e") == base_lower)
    {
        return Some("s".to_string());
    }
    None
}

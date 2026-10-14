//! Legal spelling supplement and confusion-pair proofing.

use serde_json::json;
use wordcraft_doc::Document;
use wordcraft_engine::Session;
use wordcraft_proof::{ModuleEn, Options, ProofLang, check_spelling_with_module, check_text_with, is_correct_with_module};

use crate::install;
use crate::proofing::{confusion_issues, legal_words};
use crate::tests::{law, run};

#[test]
fn legal_words_are_sorted_and_nonempty() {
    let words = legal_words();
    assert!(words.len() > 40);
    assert!(words.windows(2).all(|p| p[0] < p[1]), "sorted, no duplicates");
}

#[test]
fn legal_terms_spell_check_depends_on_lawcraft() {
    let opts = Options::default();
    assert!(!is_correct_with_module("rsmo", &opts, wordcraft_proof::ModuleEn::default()));
    let mut s = law();
    assert!(is_correct_with_module("rsmo", &opts, s.module_en));
    assert!(is_correct_with_module("tortfeasor", &opts, s.module_en));
    s.module_en = wordcraft_proof::ModuleEn::default();
    assert!(!is_correct_with_module("rsmo", &opts, s.module_en));
    assert_eq!(install(&mut s), 0);
    assert!(is_correct_with_module("rsmo", &opts, s.module_en));
}

/// Reporter and court abbreviations (`S.W.3d`, `F.4th`, `Mo. App.`) reach proofing through the
/// real spelling path: the tokenizer splits them at their periods, and the resulting pieces are
/// recognised — single letters and digit-bearing tokens by the defaults, the rest by the
/// dictionary — so none is flagged. `RSMo`, a single token the base dictionary does not have, is
/// the term the LawCraft list itself supplies: quiet with the module on, flagged without it.
#[test]
fn reporter_and_court_abbreviations_spell_clean() {
    let opts = Options::default();
    let s = law();
    for text in ["S.W.3d 679", "F.4th 1184", "Mo. App. E.D. 2020"] {
        let issues = check_spelling_with_module(text, &opts, s.module_en);
        assert!(issues.is_empty(), "LawCraft should not flag `{text}`: {issues:?}");
    }
    assert!(!is_correct_with_module("rsmo", &opts, ModuleEn::default()), "RSMo is not plain English");
    assert!(is_correct_with_module("rsmo", &opts, s.module_en), "the legal list supplies RSMo");
}

#[test]
fn law_proofing_does_not_leak_between_sessions() {
    let opts = Options::default();
    let enabled = law();
    let mut disabled = Session::new(Document::new());
    assert!(!disabled.module_enabled(crate::MODULE));
    assert_eq!(install(&mut disabled), 0);
    assert!(is_correct_with_module("rsmo", &opts, enabled.module_en));
    assert!(!is_correct_with_module("rsmo", &opts, disabled.module_en));
    let text = "The tenant claimed averse possession.";
    let on = check_text_with(text, &[(0..text.len(), ProofLang::En)], &opts, true, enabled.module_en);
    assert!(on.iter().any(|i| i.message.contains("adverse")));
    let off = check_text_with(text, &[(0..text.len(), ProofLang::En)], &opts, true, disabled.module_en);
    assert!(off.is_empty());
}

#[test]
fn law_words_list_command() {
    let mut s = law();
    let list = run(&mut s, "law.words.list", json!({}));
    let words: Vec<&str> = list.as_array().expect("array").iter().filter_map(|v| v.as_str()).collect();
    assert!(words.contains(&"demurrer"));
    assert!(words.windows(2).all(|p| p[0] < p[1]));
}

#[test]
fn confusion_check_command_finds_averse_adverse() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "The tenant claimed averse possession."}));
    let r = run(&mut s, "law.proof.confusions", json!({"store": false}));
    let findings = r["findings"].as_array().expect("findings");
    assert!(findings.iter().any(|f| f["message"].as_str().is_some_and(|m| m.contains("adverse"))));
}

/// The confusion check reaches the real Review › Proofing path: an enabled session's module hook
/// is carried on `Session::module_en` into layout proofing, so `review.issues` (what the Review
/// pane and F7 read) reports it — and an uninstalled session does not.
#[test]
fn confusion_issues_hook_into_review_proofing() {
    let text = "The tenant claimed averse possession.";
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": text}));
    let issues = run(&mut s, "review.issues", json!({}));
    assert!(
        issues.as_array().is_some_and(|a| a.iter().any(|i| i["message"].as_str().is_some_and(|m| m.contains("adverse")))),
        "review.issues should report the confusion: {issues}"
    );

    let mut off = Session::new(Document::new());
    assert_eq!(install(&mut off), 0);
    run(&mut off, "text.insert", json!({"text": text}));
    let issues = run(&mut off, "review.issues", json!({}));
    assert!(issues.as_array().is_some_and(|a| a.is_empty()), "without LawCraft the same text is quiet: {issues}");
}

/// A firing case must name the exact word highlighted and the exact replacement offered, so a
/// wrong suggestion can't satisfy coverage by merely producing some finding.
macro_rules! confusion_pair {
    ($name:ident, $fires:expr, $quiet:expr, $wrong:expr, $right:expr) => {
        #[test]
        fn $name() {
            let fired = confusion_issues($fires);
            assert_eq!(fired.len(), 1, "expected one finding in: {} → {fired:?}", $fires);
            let issue = &fired[0];
            assert_eq!($fires.get(issue.start..issue.end), Some($wrong), "highlighted word in: {}", $fires);
            assert_eq!(issue.suggestions, vec![$right.to_string()], "replacement in: {}", $fires);
            let quiet = confusion_issues($quiet);
            assert!(quiet.is_empty(), "unexpected finding in: {} → {:?}", $quiet, quiet);
        }
    };
}

/// Phrase matching must not join tokens across a sentence-ending period.
#[test]
fn confusion_phrase_does_not_cross_sentence_boundary() {
    let quiet = "They remained averse. Possession was never disputed.";
    assert!(confusion_issues(quiet).is_empty(), "cross-sentence match: {quiet}");
    let fired = confusion_issues("The tenant claimed averse possession.");
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].suggestions, vec!["adverse".to_string()]);
}

/// #5 review: only plain whitespace may join the words of a confusion phrase. Intervening
/// punctuation (semicolon, colon, comma, parentheses, dash, quotes) must break the match, so
/// `averse; possession` never fires even though the words are adjacent in token order.
#[test]
fn confusion_phrase_requires_plain_whitespace_between_words() {
    for text in [
        "They remained averse; possession was never disputed.",
        "They remained averse: possession was never disputed.",
        "They remained averse, possession was never disputed.",
        "They remained averse (possession was never disputed).",
        "They remained averse - possession was never disputed.",
        "They remained averse \"possession\" was never disputed.",
        "They remained averse-possession was never disputed.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "punctuation joined a phrase in: {text} → {issues:?}");
    }
    let fired = confusion_issues("The tenant claimed adverse possession.");
    assert!(fired.is_empty(), "correct usage must stay quiet: {fired:?}");
}

/// The suggestion keeps the case of the wrong word.
#[test]
fn confusion_suggestion_matches_case() {
    let lower = confusion_issues("The tenant claimed averse possession.");
    assert_eq!(lower.len(), 1);
    assert_eq!(lower[0].suggestions, vec!["adverse".to_string()]);
    let upper = confusion_issues("Averse possession was claimed.");
    assert_eq!(upper.len(), 1);
    assert_eq!(upper[0].suggestions, vec!["Adverse".to_string()]);
}

confusion_pair!(adverse_averse, "The tenant claimed averse possession.", "The investor was averse to risk.", "averse", "adverse");

/// #5 review: `principle place of business` is removed. "Under this principle" is an introductory
/// prepositional phrase whose comma is optional, so this correct sentence must stay quiet.
#[test]
fn removed_principle_place_of_business_stays_quiet() {
    for text in [
        "Under this principle place of business determines jurisdiction.",
        "Under this principle, place of business determines jurisdiction.",
        "The principle place so designated governs.",
        "The principal place of business is in St. Louis.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// Removed phrases must stay quiet (regression guard).
#[test]
fn removed_confusion_phrases_stay_quiet() {
    for text in [
        // `statue of limitations` is ambiguous: a statue may depict limitations, so the phrase is
        // removed even though the legal usage is far more common.
        "The sculptor carved a statue of limitations, depicting the constraints imposed on human freedom.",
        // `statue of frauds` is ambiguous (`frauds` = impostors), so the phrase is removed.
        "The sculptor carved a statue of frauds who had posed as philanthropists.",
        "The principle claim for damages was dismissed.",
        "The principle amount remains unpaid.",
        "The affect of the ruling was clear.",
        "The order will affect a change in procedure.",
        "This is binding precedence in our circuit.",
        "We conferred with opposing council before trial.",
        "Counsel tried to illicit testimony on redirect.",
        "There are three discreet issues before us.",
        "We assessed the effect the outcome had on the parties.",
        "The physician testified that the regulation will proscribe medication advertising to minors.",
        // `council for the <role>` is ambiguous: here `council` correctly names the
        // municipality's governing body, with the role word modifying `municipality`.
        "The city council for the plaintiff municipality approved the settlement.",
        "The city council for the defendant municipality approved the settlement.",
        "The city council for the appellant municipality approved the settlement.",
        "The city council for the respondent municipality approved the settlement.",
        "The city council for the petitioner municipality approved the settlement.",
        "The city council for the movant municipality approved the settlement.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// #5 review: the mixed-case `RSMo` users actually type is clean with LawCraft on, through the
/// same paragraph spelling path the editor uses, and flagged when it is off.
#[test]
fn mixed_case_rsmo_spells_clean_only_with_lawcraft() {
    let opts = Options::default();
    let text = "The RSMo governs this appeal under S.W.3d.";
    let on = law();
    let issues = check_spelling_with_module(text, &opts, on.module_en);
    assert!(issues.is_empty(), "LawCraft should not flag `{text}`: {issues:?}");
    let off = check_spelling_with_module(text, &opts, ModuleEn::default());
    assert!(off.iter().any(|i| text.get(i.start..i.end) == Some("RSMo")), "without LawCraft, RSMo is flagged: {off:?}");
}

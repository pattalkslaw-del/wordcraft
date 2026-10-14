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
    let text = "The statue of limitations has run.";
    let on = check_text_with(text, &[(0..text.len(), ProofLang::En)], &opts, true, enabled.module_en);
    assert!(on.iter().any(|i| i.message.contains("statute")));
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
fn confusion_check_command_finds_statute_statue() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "The statue of limitations has run."}));
    let r = run(&mut s, "law.proof.confusions", json!({"store": false}));
    let findings = r["findings"].as_array().expect("findings");
    assert!(findings.iter().any(|f| f["message"].as_str().is_some_and(|m| m.contains("statute"))));
}

/// The confusion check reaches the real Review › Proofing path: an enabled session's module hook
/// is carried on `Session::module_en` into layout proofing, so `review.issues` (what the Review
/// pane and F7 read) reports it — and an uninstalled session does not.
#[test]
fn confusion_issues_hook_into_review_proofing() {
    let text = "The statue of limitations has run.";
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": text}));
    let issues = run(&mut s, "review.issues", json!({}));
    assert!(
        issues.as_array().is_some_and(|a| a.iter().any(|i| i["message"].as_str().is_some_and(|m| m.contains("statute")))),
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

confusion_pair!(statute_statue, "The statue of limitations has run.", "A bronze statue stands in the plaza.", "statue", "statute");

/// Phrase matching must not join tokens across a sentence-ending period.
#[test]
fn confusion_phrase_does_not_cross_sentence_boundary() {
    let quiet = "We admired the statue. Of limitations, none applied.";
    assert!(confusion_issues(quiet).is_empty(), "cross-sentence match: {quiet}");
    let fired = confusion_issues("The statue of limitations has run.");
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].suggestions, vec!["statute".to_string()]);
}

/// #5 review: only plain whitespace may join the words of a confusion phrase. Intervening
/// punctuation (semicolon, colon, comma, parentheses, dash, quotes) must break the match, so
/// `statue; of limitations` never fires even though the words are adjacent in token order.
#[test]
fn confusion_phrase_requires_plain_whitespace_between_words() {
    for text in [
        "We admired the statue; of limitations, none applied.",
        "We admired the statue: of limitations, none applied.",
        "We admired the statue, of limitations, none applied.",
        "We admired the statue (of limitations) at the park.",
        "We admired the statue - of limitations none applied.",
        "We admired the statue \"of limitations\" at the park.",
        "The statue of-limitations defense failed.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "punctuation joined a phrase in: {text} → {issues:?}");
    }
    let fired = confusion_issues("The statute of limitations has run.");
    assert!(fired.is_empty(), "correct usage must stay quiet: {fired:?}");
}

/// Plural/possessive endings on the wrong word are preserved in the suggestion.
#[test]
fn confusion_suggestion_preserves_inflection() {
    let fired = confusion_issues("The statues of limitations have run.");
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].start..fired[0].end, 4..11);
    assert_eq!(text_slice("The statues of limitations have run.", fired[0].start, fired[0].end), "statues");
    assert_eq!(fired[0].suggestions, vec!["statutes".to_string()]);
    let possessive = confusion_issues("The statue's of limitations defense failed.");
    assert_eq!(possessive.len(), 1);
    assert_eq!(possessive[0].suggestions, vec!["statute's".to_string()]);
}

fn text_slice(text: &str, start: usize, end: usize) -> &str {
    text.get(start..end).unwrap_or("")
}
confusion_pair!(statute_frauds, "The statue of frauds bars oral agreements.", "A bronze statue stands in the plaza.", "statue", "statute");
/// #5 review: an inflected *context* word must not match. The removed `principle issue` entry
/// fired on "The principle issues from natural law." because the old matcher accepted `issues` as
/// a plural of `issue`, suggesting `principal` for a correct noun. Every phrase now requires its
/// context words exactly, so a following verb (`issues`, `claims`, `amounts`, `places`) stays
/// quiet; this is that regression.
#[test]
fn principle_as_noun_before_a_verb_is_quiet() {
    for text in [
        "The principle issues from natural law.",
        "A legal principle issues from the cases.",
        "The principle claims our attention.",
        "The principle amounts to nothing more than fairness.",
        "The principle places the burden on the movant.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}
confusion_pair!(
    principal_principle_claim,
    "The principle claim for damages was dismissed.",
    "A basic principle guides the analysis.",
    "principle",
    "principal"
);
confusion_pair!(
    principal_principle_amount,
    "The principle amount remains unpaid.",
    "A basic principle guides the analysis.",
    "principle",
    "principal"
);
confusion_pair!(
    principal_place_of_business,
    "The principle place of business is in St. Louis.",
    "The principal amount is unpaid.",
    "principle",
    "principal"
);
confusion_pair!(principal_of_law, "The principal of law is settled.", "The principal amount is unpaid.", "principal", "principle");
confusion_pair!(
    counsel_council_plaintiff,
    "The council for the plaintiff filed a motion.",
    "The city council for the district met yesterday.",
    "council",
    "counsel"
);
confusion_pair!(
    counsel_council_defendant,
    "The council for the defendant objected.",
    "The city council for the district met yesterday.",
    "council",
    "counsel"
);
confusion_pair!(
    counsel_council_appellant,
    "The council for the appellant argued first.",
    "The city council for the district met yesterday.",
    "council",
    "counsel"
);
confusion_pair!(
    counsel_council_respondent,
    "The council for the respondent replied.",
    "The city council for the district met yesterday.",
    "council",
    "counsel"
);
confusion_pair!(
    counsel_council,
    "Attorney council of record filed a brief.",
    "The city council for the district met yesterday.",
    "council",
    "counsel"
);
confusion_pair!(
    opposing_council,
    "We conferred with opposing council before trial.",
    "The city council voted for the ordinance.",
    "council",
    "counsel"
);
confusion_pair!(affect_effect, "The affect of the ruling was clear.", "The new rule will affect the outcome.", "affect", "effect");
confusion_pair!(
    affect_a_change,
    "The order will affect a change in procedure.",
    "The parties hope to effect a change in procedure.",
    "affect",
    "effect"
);
confusion_pair!(
    precedent_precedence,
    "This is binding precedence in our circuit.",
    "Federal law takes precedence over state law.",
    "precedence",
    "precedent"
);
/// #5 review: `precedence` correctly means priority, so a phrase like `controlling precedence`
/// is ambiguous and must stay quiet; only the fixed idiom `binding precedence` (which suggests
/// `precedent`) is an entry. This is the regression for the removed `controlling precedence` rule.
#[test]
fn controlling_precedence_is_quiet() {
    for text in [
        "The scheduling order contains rules controlling precedence among competing motions.",
        "The court explained the rules controlling precedence among competing motions.",
        "The statute takes precedence over the rule.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// #5 review: `proscribe` correctly means to forbid, and a subject that can forbid medication
/// (`Doctors proscribe medication that is contraindicated`) makes any `doctor proscribe
/// medication` phrase ambiguous, so that entry was removed. `proscribe` in these correct
/// sentences must stay quiet.
#[test]
fn proscribe_medication_usage_is_quiet() {
    for text in [
        "Doctors proscribe medication that is contraindicated.",
        "The physician testified that the regulation will proscribe medication advertising to minors.",
        "The regulation will proscribe medication advertising to minors.",
        "The statute will proscribe medication by unlicensed sellers.",
        "The statute will proscribe medication for nonmedical use.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

confusion_pair!(adverse_averse, "The tenant claimed averse possession.", "The investor was averse to risk.", "averse", "adverse");
confusion_pair!(adverse_to_risk, "The clause is adverse to risk.", "The investor was averse to risk.", "adverse", "averse");
confusion_pair!(adverse_to_doing, "She is adverse to doing business there.", "The investor was averse to risk.", "adverse", "averse");
confusion_pair!(
    elicit_illicit,
    "Counsel tried to illicit testimony on redirect.",
    "Counsel tried to elicit testimony on redirect.",
    "illicit",
    "elicit"
);
confusion_pair!(
    elicit_illicit_response,
    "The witness could illicit a response on cross-examination.",
    "The cross-examiner sought a candid response.",
    "illicit",
    "elicit"
);
/// #5 review: a "full compliment of the judge" is correct praise, so the old `full compliment of`
/// entry was ambiguous and was removed. `compliment` (praise) and `complement` (a complete set)
/// used correctly must all stay quiet: there is no unambiguous `compliment` phrase.
#[test]
fn full_compliment_and_complement_are_quiet() {
    for text in [
        "The witness recalled the full compliment of the judge: 'Your work was excellent.'",
        "The court now has a full complement of judges.",
        "The court paid the witness a compliment after the hearing.",
        "Her closing remarks were a fitting complement to the evidence.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// #5 review: praising a person is `compliment`; the old `compliment the record` entry fired on
/// this correct sentence and had to be replaced with an unambiguous phrase.
#[test]
fn compliment_the_record_keeper_is_quiet() {
    for text in [
        "Counsel will compliment the record keeper after the hearing.",
        "The judge may compliment counsel on the argument.",
        "The panel will compliment the clerk for prompt service.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}
confusion_pair!(
    discrete_discreet,
    "There are three discreet issues before us.",
    "The agency opened a discrete investigation into each transaction.",
    "discreet",
    "discrete"
);

/// `effect` as a verb is legitimate ("bring about"), so a bare `effect the outcome` is ambiguous
/// and must stay quiet rather than guess at `affect`.
#[test]
fn effect_the_outcome_is_quiet() {
    for text in ["The new rule will effect the outcome the parties sought.", "The court's order will effect the outcome of the settlement."] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// `effect` as a noun before another noun must not suggest `affect`.
#[test]
fn effect_as_noun_before_outcome_is_quiet() {
    let text = "We assessed the effect the outcome had on the parties.";
    let issues = confusion_issues(text);
    assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
}

/// Correct governing-body `council` and affirmative `prescribe` must stay quiet.
#[test]
fn correct_council_and_prescribe_contexts_are_quiet() {
    for text in [
        "The city council for the district met yesterday.",
        "The county council for the region approved the plan.",
        "The rules prescribe such conduct by counsel.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// Correct verb `complement` must stay quiet.
#[test]
fn correct_complement_contexts_are_quiet() {
    for text in ["The new tribunal will complement the court.", "These judges complement the court well."] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// `discrete inquiry` and `discrete investigation` are correct and must not suggest `discreet`.
#[test]
fn discrete_inquiry_and_investigation_are_quiet() {
    for text in [
        "The court made a discrete inquiry into venue.",
        "The agency opened a discrete investigation into each transaction.",
        "The court ran a discrete investigation of each charge.",
        "The firm conducted a discreet investigation.",
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

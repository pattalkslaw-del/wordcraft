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
confusion_pair!(statute_frauds, "The statue of frauds bars oral agreements.", "A bronze statue stands in the plaza.", "statue", "statute");
confusion_pair!(principal_principle_issue, "The principle issue is liability.", "A moral principle guides the court.", "principle", "principal");
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
confusion_pair!(
    controlling_precedence,
    "This decision is controlling precedence here.",
    "The statute takes precedence over the rule.",
    "precedence",
    "precedent"
);
confusion_pair!(
    prescribe_proscribe,
    "The doctor proscribe medication for the patient.",
    "The rules prescribe such conduct by counsel.",
    "proscribe",
    "prescribe"
);

/// Exact-phrase check: forbidding medication advertising is `proscribe`, not a doctor's `prescribe`.
#[test]
fn proscribe_medication_advertising_is_quiet() {
    for text in [
        "The physician testified that the regulation will proscribe medication advertising to minors.",
        "The regulation will proscribe medication advertising to minors.",
        "The statute will proscribe medication by unlicensed sellers.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

confusion_pair!(adverse_averse, "The tenant claimed averse possession.", "The investor was averse to risk.", "averse", "adverse");
confusion_pair!(adverse_to_risk, "The clause is adverse to risk.", "The investor was averse to risk.", "adverse", "averse");
confusion_pair!(adverse_to_doing, "She is adverse to doing business there.", "The investor was averse to risk.", "adverse", "averse");
confusion_pair!(elicit_illicit, "The deposition will illicit testimony about the contract.", "Police seized illicit drugs.", "illicit", "elicit");
confusion_pair!(
    elicit_illicit_response,
    "The witness could illicit a response on cross-examination.",
    "The cross-examiner sought a candid response.",
    "illicit",
    "elicit"
);
confusion_pair!(complement_compliment, "The exhibit compliment the record.", "She compliment the judge after argument.", "compliment", "complement");
confusion_pair!(
    discrete_discreet,
    "There are three discreet issues before us.",
    "The agency opened a discrete investigation into each transaction.",
    "discreet",
    "discrete"
);
confusion_pair!(effect_affect_outcome, "The new rule will effect the outcome.", "The new rule will affect the outcome.", "effect", "affect");

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

/// Correct adjective `illicit` and verb `complement` must stay quiet (no bare `illicit testimony` rule).
#[test]
fn correct_illicit_and_complement_contexts_are_quiet() {
    for text in [
        "The motion challenges illicit testimony.",
        "The court excluded illicit testimony at trial.",
        "The objection to illicit testimony was sustained.",
        "The new tribunal will complement the court.",
        "These judges complement the court well.",
    ] {
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

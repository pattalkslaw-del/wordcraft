//! Legal spelling supplement and confusion-pair proofing.

use serde_json::json;
use wordcraft_doc::Document;
use wordcraft_engine::Session;
use wordcraft_proof::{Options, ProofLang, check_text_with, is_correct_with_module};

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
confusion_pair!(principal_principle_issue, "The principle issue is liability.", "A moral principle guides the court.", "principle", "principal");
confusion_pair!(
    counsel_council,
    "Attorney council of record filed a brief.",
    "The city council for the district met yesterday.",
    "council",
    "counsel"
);
confusion_pair!(
    counsel_council_ordinance,
    "The council of record filed an appearance.",
    "The city council voted for the ordinance.",
    "council",
    "counsel"
);
confusion_pair!(affect_effect, "The affect of the ruling was clear.", "The new rule will affect the outcome.", "affect", "effect");
confusion_pair!(affect_statute, "The affect of the ruling was clear.", "The rule will affect enforcement of the statute.", "affect", "effect");
confusion_pair!(
    precedent_precedence,
    "This is binding precedence in our circuit.",
    "Federal law takes precedence over state law.",
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

/// #5 review: correct wording must not be flagged. A governing-body `council` — even "council for
/// the district" — and a rule that affirmatively `prescribe`s conduct are both correct; adjacency
/// alone doesn't make the word likely wrong. The clear `council of record` and `proscribe
/// medication` cases still fire (see the pairs above).
#[test]
fn correct_council_and_prescribe_contexts_are_quiet() {
    for text in [
        "The city council for the district met yesterday.",
        "The county council for the region approved the plan.",
        "The rules prescribe such conduct by counsel.",
        "The statute prescribe such conduct.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}
confusion_pair!(adverse_averse, "The tenant claimed averse possession.", "The investor was averse to risk.", "averse", "adverse");
confusion_pair!(elicit_illicit, "The lawyer attempted to illicit testimony from the witness.", "Police seized illicit drugs.", "illicit", "elicit");
confusion_pair!(complement_compliment, "The exhibit compliment the record.", "She compliment the judge after argument.", "compliment", "complement");
confusion_pair!(discrete_discreet, "There are three discreet issues before us.", "A discreet investigation continued.", "discreet", "discrete");

/// #5 review: correct `illicit` (an adjective meaning unlawful) and correct `complement` (to
/// complete or balance) must not be flagged. Only a verb context — after an auxiliary or `to` —
/// makes `illicit` likely wrong, and `complement the court` is a valid object; adjacency alone
/// doesn't establish a mistake. The clear `to illicit testimony` and `compliment the record` cases
/// still fire (see the pairs above).
#[test]
fn correct_illicit_and_complement_contexts_are_quiet() {
    for text in [
        "The motion challenges illicit testimony.",
        "The court excluded illicit testimony at trial.",
        "The new tribunal will complement the court.",
        "These judges complement the court well.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

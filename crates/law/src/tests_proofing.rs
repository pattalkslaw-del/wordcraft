//! Legal spelling supplement and confusion-pair proofing.

use serde_json::json;
use wordcraft_proof::{IssueKind, ProofLang, check_text_with, is_correct};

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
    proofing_off();
    assert!(!is_correct("rsmo"), "Moby does not include the RSMo abbreviation");
    let mut s = law();
    assert!(is_correct("rsmo"));
    assert!(is_correct("tortfeasor"));
    proofing_off();
    assert!(!is_correct("rsmo"));
    // A fresh install restores the supplement (commands stay registered).
    assert_eq!(install(&mut s), 0);
    assert!(is_correct("rsmo"));
}

fn proofing_off() {
    crate::proofing::uninstall();
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

#[test]
fn confusion_issues_hook_into_review_proofing() {
    let _s = law();
    let text = "The statue of limitations has run.";
    let issues = check_text_with(text, &[(0..text.len(), ProofLang::En)], &Default::default(), true);
    assert!(issues.iter().any(|i| i.kind == IssueKind::Grammar && i.message.contains("statute")));
}

macro_rules! confusion_pair {
    ($name:ident, $fires:expr, $quiet:expr) => {
        #[test]
        fn $name() {
            let fired = confusion_issues($fires);
            assert!(!fired.is_empty(), "expected a finding in: {}", $fires);
            let quiet = confusion_issues($quiet);
            assert!(quiet.is_empty(), "unexpected finding in: {} → {:?}", $quiet, quiet);
        }
    };
}

confusion_pair!(statute_statue, "The statue of limitations has run.", "A bronze statue stands in the plaza.");
confusion_pair!(principal_principle_issue, "The principle issue is liability.", "A moral principle guides the court.");
confusion_pair!(counsel_council, "Attorney council for the defendant filed a brief.", "The city council met last night.");
confusion_pair!(affect_effect, "The affect of the ruling was clear.", "The new rule will affect the outcome.");
confusion_pair!(precedent_precedence, "This is binding precedence in our circuit.", "Federal law takes precedence over state law.");
confusion_pair!(prescribe_proscribe, "The doctor proscribe medication for the patient.", "The regulation proscribe such conduct.");
confusion_pair!(proscribe_prescribe, "The rule prescribe such conduct by counsel.", "The physician prescribe antibiotics.");
confusion_pair!(adverse_averse, "The tenant claimed averse possession.", "The investor was averse to risk.");
confusion_pair!(elicit_illicit, "Counsel illicit testimony about the contract.", "Police seized illicit drugs.");
confusion_pair!(complement_compliment, "The exhibit compliment the record.", "She compliment the judge after argument.");
confusion_pair!(discrete_discreet, "There are three discreet issues before us.", "A discreet investigation continued.");

//! LawCraft: court profiles for legal documents, as a WordCraft module.
//!
//! A court profile is a data file — one per court, JSON with a versioned schema — that says how
//! a document of a given type is put together: its **sections** (the skeleton, with the caption
//! and its fields), its **styles** (page settings, named WordCraft styles, line numbers) and its
//! **checks** (word limits and the like). Profiles are loaded, listed, created, updated,
//! validated, approved, imported, exported and applied through commands, so the ribbon, the
//! command palette, the CLI, the JSON control channel and the MCP server all reach them.
//!
//! Two profiles ship verified: a Missouri appellate brief (Rules 84.03, 84.04, 84.06) and a
//! federal appellate brief (FRAP 32). Every rule in them records the rule it comes from and when
//! it took effect. Other courts are entered by users or their agents — and anything an agent
//! writes is saved unverified until a person approves it.
//!
//! Nothing here formats a document on its own: applying a profile runs WordCraft's own style and
//! layout commands (`styles.create`, `styles.modify`, `layout.size`, `layout.margins`,
//! `layout.lineNumbers`, `insert.pageNumber`). Legal *rules* live in the profile files; the legal
//! *logic* that isn't formatting (checks, citations, proofing) comes in later pieces.
//!
//! The module is off until the user turns it on (`law.enabled`, File › Options › Add-ins), and a
//! host installs the commands only then — see [`install`].
//!
//! # Example
//!
//! ```
//! use serde_json::json;
//! use wordcraft_engine::Session;
//!
//! let mut session = Session::new(wordcraft_doc::Document::new());
//! session.prefs.modules.insert("law".into(), true);
//! wordcraft_law::install(&mut session);
//! let profiles = session.run("law.profiles.list", &json!({})).expect("list");
//! assert_eq!(profiles.as_array().map(Vec::len), Some(2));
//! ```

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod apply;
pub mod checks;
pub mod citation_pack;
pub mod citations;
pub mod cmd;
pub mod profile;
pub mod store;
mod toa;

use wordcraft_engine::Session;

pub use cmd::{set_profiles_dir, settings_dir, specs, store};
pub use profile::{Profile, SCHEMA};
pub use store::LawError;

/// The module id the settings use (`law.enabled`).
pub const MODULE: &str = "law";

/// Turn the module on for a session: installs the `law.*` commands when `law.enabled` is set in
/// the editing preferences, and leaves the session alone when it isn't. A host calls this once,
/// after it has read the user's preferences, and points the profile store at the user's own
/// profiles folder first ([`set_profiles_dir`]).
pub fn install(session: &mut Session) -> usize {
    if !session.module_enabled(MODULE) {
        return 0;
    }
    checks::register_document_hook();
    // A host may open the file *before* it installs LawCraft (`Session::new(doc)`), so the
    // document-loaded hook never runs for it. Restore the applied profile's export checks from
    // the document's own metadata at installation, the same way `set_document` would.
    checks::restore_export_registration(session);
    session.install_commands(specs())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_checks;

#[cfg(test)]
mod tests_citations;

#[cfg(test)]
mod tests_toa;

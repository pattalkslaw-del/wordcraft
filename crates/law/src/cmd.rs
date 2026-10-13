//! The LawCraft commands: `law.profiles.*`.
//!
//! Everything a person or an agent can do with court profiles is a command, so the ribbon, the
//! command palette, shortcuts, the CLI, the JSON control channel and the MCP server all reach
//! them the same way. The commands hold no legal rules themselves: they read, write and apply
//! the profile files ([`crate::profile`], [`crate::store`], [`crate::apply`]).
//!
//! A profile created or updated through these commands is always saved unverified: only
//! `law.profiles.approve`, a person's action, verifies one.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde_json::{Value, json};
use wordcraft_engine::{CmdError, CommandSpec, p};

use crate::apply;
use crate::profile::{Patch, Profile};
use crate::store::{LawError, Store};

/// Where the commands live in the ribbon.
const LOC: &str = "Law › Court Profiles";

/// The profiles WordCraft knows: the ones it ships and the user's own.
static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

/// The shared store.
pub fn store() -> MutexGuard<'static, Store> {
    STORE.get_or_init(|| Mutex::new(Store::default())).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Point the profile store at the user's profiles folder. The host calls this once at start-up;
/// without one, the bundled profiles and anything saved in this run are all there is.
pub fn set_profiles_dir(dir: Option<PathBuf>) {
    store().set_dir(dir);
}

/// The user's profiles folder inside WordCraft's settings folder (`<settings>/law/profiles`).
pub fn settings_dir() -> Option<PathBuf> {
    wordcraft_control_key::settings_dir().map(|d| d.join("law").join("profiles"))
}

/// The commands, for [`wordcraft_engine::Session::install_commands`].
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("law.profiles.list", "Court Profiles", LOC, |_, _| {
            let store = store();
            Ok(Value::Array(store.list().iter().map(|p| p.summary()).collect()))
        })
        .params(r#"{} → [{id, name, jurisdiction, verified, citationPack, docs: [document type]}]"#)
        .pure(),
        CommandSpec::new("law.profiles.get", "Get Court Profile", LOC, |_, v| {
            let id = p::req_str(v, "id")?;
            store().get(id).map(|p| p.to_value()).ok_or_else(|| not_found(id))
        })
        .params(r#"{"id": string} → the profile (see law.profiles.create for the shape)"#)
        .pure(),
        CommandSpec::new("law.profiles.create", "Create Court Profile", LOC, |_, v| {
            let mut p = from_value(v)?;
            if store().get(&p.id).is_some() {
                return Err(CmdError::Failed(format!("a court profile `{}` already exists (use law.profiles.update)", p.id)));
            }
            // Whoever wrote it, it is unverified until a person approves it.
            store().save(&mut p).map_err(invalid)?;
            Ok(p.to_value())
        })
        .params(
            r#"the profile: {"schema": 1, "id": string, "name": string, "jurisdiction": string, "verified"?: bool, "citationPack"?: string, "docs": {<document type>: {"label"?, "page": Page, "lineNumbers"?: LineNumbers, "pageNumbers"?: PageNumbers, "styles": [Style], "sections": [Section], "checks": [Check], "source": string, "effective"?: "YYYY-MM-DD"}}}"#,
        )
        .pure(),
        CommandSpec::new("law.profiles.update", "Update Court Profile", LOC, |_, v| {
            let patch = Patch::from_value(v.clone()).map_err(|bad| invalid(LawError::invalid(&bad)))?;
            let id = patch.id.as_deref().ok_or_else(|| CmdError::Params("`id` (string) is required".into()))?.to_string();
            let Some(base) = store().get(&id).cloned() else { return Err(not_found(&id)) };
            // A changed profile is unverified again, whoever changed it. Nothing is written
            // when the result isn't a valid profile.
            let mut p = patch.apply(&base);
            store().save(&mut p).map_err(invalid)?;
            Ok(p.to_value())
        })
        .params(r#"{"id": string, "name"?, "jurisdiction"?, "citationPack"?, "docs"?} — the fields given replace the stored profile's; the rest stay"#)
        .pure(),
        CommandSpec::new("law.profiles.validate", "Validate Court Profile", LOC, |_, v| {
            let p = from_value(v)?;
            Ok(json!({"ok": true, "id": p.id, "name": p.name, "docs": p.doc_types()}))
        })
        .params(r#"a profile as law.profiles.create takes it (or {"path": string} for a file) → {"ok": true, …}; every problem found is reported at once, and nothing is saved"#)
        .pure(),
        CommandSpec::new("law.profiles.approve", "Approve Court Profile", LOC, |_, v| {
            let id = p::req_str(v, "id")?;
            // A person's action: the only thing that verifies a profile.
            Ok(store().approve(id).map_err(invalid)?.to_value())
        })
        .params(r#"{"id": string} → the profile, verified"#)
        .pure(),
        CommandSpec::new("law.profiles.import", "Import Court Profile", LOC, |_, v| {
            let path = p::req_str(v, "path")?;
            let mut p = Store::read_file(std::path::Path::new(path)).map_err(invalid)?;
            // Automation never grants verification: only a person importing outside MCP or a
            // confined agent session may keep or set the verified flag from the file. A confined
            // agent may reach a file that says `"verified": true` (or pass the override) without
            // the automation marker, so confinement counts too.
            if wordcraft_engine::file_access::automation_invoked() || wordcraft_engine::file_access::automation_confined() {
                p.verified = false;
            } else if let Some(verified) = p::bool(v, "verified") {
                p.verified = verified;
            }
            store().insert(p.clone()).map_err(invalid)?;
            Ok(p.to_value())
        })
        .params(r#"{"path": string, "verified"?: bool (outside automation only: override the file's flag)} → the profile as it is now in the store; importing under MCP or a confined agent session always saves it unverified"#)
        .pure(),
        CommandSpec::new("law.profiles.export", "Export Court Profile", LOC, |_, v| {
            let id = p::req_str(v, "id")?;
            let path = p::req_str(v, "path")?;
            let Some(p) = store().get(id).cloned() else { return Err(not_found(id)) };
            Store::write_file(std::path::Path::new(path), &p).map_err(invalid)?;
            Ok(json!({"path": path, "id": id}))
        })
        .params(r#"{"id": string, "path": string} → {"path", "id"}"#)
        .pure(),
        CommandSpec::new("law.profiles.apply", "Apply Court Profile", LOC, |s, v| {
            let id = p::req_str(v, "id")?;
            let Some(p) = store().get(id).cloned() else { return Err(not_found(id)) };
            let doc = match p::str(v, "doc") {
                Some(d) => d.to_string(),
                // One document type goes without saying; several, and the caller names one.
                None => {
                    let mut types = p.doc_types();
                    if types.len() != 1 {
                        return Err(CmdError::Params(format!("`doc` is required: `{}` has {}", p.name, types.join(", "))));
                    }
                    types.remove(0).to_string()
                }
            };
            // Usable even unverified: the flag says whether a person checked the rules.
            apply::apply(s, &p, &doc).map_err(|e| CmdError::Failed(e.to_string()))
        })
        .params(r#"{"id": string, "doc"?: string (a document type of the profile)} → {"profile", "doc", "applied": [command], "styles": [style id]}"#),
    ]
}

fn not_found(id: &str) -> CmdError {
    CmdError::Failed(format!("no court profile `{id}`"))
}

/// A command parameter that isn't a profile: every problem, reported together.
fn invalid(e: LawError) -> CmdError {
    match e {
        LawError::Invalid(problems) => CmdError::Params(format!("the profile is not valid:\n  {problems}")),
        LawError::HumanOnly(msg) => CmdError::Disabled(msg),
        other => CmdError::Failed(other.to_string()),
    }
}

/// Read a profile out of command parameters (or a file, with `{"path": …}`).
fn from_value(v: &Value) -> Result<Profile, CmdError> {
    if let Some(path) = p::str(v, "path") {
        return Store::read_file(std::path::Path::new(path)).map_err(invalid);
    }
    let mut v = v.clone();
    // A `path` is how a file is asked for; anything else must be the profile itself.
    if let Some(obj) = v.as_object_mut() {
        obj.remove("path");
    }
    Profile::from_value(v).map_err(|bad| invalid(LawError::invalid(&bad)))
}

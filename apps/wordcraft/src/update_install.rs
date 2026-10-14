//! One-click update handoff after a verified download (#15).
//!
//! The platform installers run only after `file.checkForUpdates` verified the download against the
//! release's published SHA-256 checksums. Each path is arranged so a failure never leaves a
//! half-installed application: the macOS bundle is staged, validated and swapped with the old
//! bundle kept until the new one is in place, and the Windows portable update stages a validated
//! copy on the install's own volume, then renames the old installation aside and restores it if the
//! new one cannot be moved into place (see [`replace_portable`]).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use wordcraft_engine::update::{InstallFn, InstallHandoff, InstallKind};

pub fn handoff() -> InstallFn {
    std::sync::Arc::new(run)
}

fn run(job: InstallHandoff) -> Result<(), String> {
    let kind = job.kind;
    let staging = private_staging_dir()?;
    let result = install_into(&job, &staging);
    // A started portable launcher owns the staging and removes it once the helper has run (success
    // or failure). Every other outcome, including a portable attempt that failed before the
    // launcher started, can drop its staging right away.
    if !(result.is_ok() && kind == InstallKind::WindowsPortableZip) {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

fn install_into(job: &InstallHandoff, staging: &Path) -> Result<(), String> {
    let file_name = safe_name(&job.file_name);
    let path = write_exclusive_file(staging, &file_name, &job.bytes)?;
    match job.kind {
        InstallKind::WindowsMsi => windows_msi(&path),
        InstallKind::WindowsPortableZip => windows_portable(&path, &job.exe_path, staging),
        InstallKind::LinuxDeb => linux_deb(&path),
        InstallKind::MacDmg | InstallKind::MacApp => mac_dmg(&path, staging, &job.exe_path),
        InstallKind::Flatpak | InstallKind::AppImage | InstallKind::Unknown => Err("This install type must use the release page.".into()),
    }
}

/// A private directory only this process can create (not a predictable shared path).
pub fn private_staging_dir() -> Result<PathBuf, String> {
    let hex = random_hex(16)?;
    let path = std::env::temp_dir().join(format!("wordcraft-update-{hex}"));
    std::fs::create_dir(&path).map_err(|e| e.to_string())?;
    // Owner-only: no other local user can read or tamper with the staged installer or the
    // downloaded bytes. `create_dir`'s ambient mode is not enough on a shared machine.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

fn random_hex(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
    let mut hex = String::with_capacity(bytes * 2);
    for b in buf {
        hex.push(hex_digit(b >> 4));
        hex.push(hex_digit(b & 0xf));
    }
    Ok(hex)
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => char::from_u32('0' as u32 + n as u32).unwrap_or('0'),
        10..=15 => char::from_u32('a' as u32 + (n - 10) as u32).unwrap_or('a'),
        _ => '0',
    }
}

/// Write bytes to a new file inside `dir`; refuses if the name already exists.
pub fn write_exclusive_file(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return Err("invalid file name".into());
    }
    let path = dir.join(name);
    let mut f = OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
    f.write_all(bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn safe_name(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '_').collect()
}

#[cfg(target_os = "windows")]
fn windows_msi(path: &Path) -> Result<(), String> {
    let status = Command::new("msiexec").args(["/i", path.to_str().ok_or("bad path")?, "/passive"]).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("msiexec exited with {status}")) }
}

#[cfg(not(target_os = "windows"))]
fn windows_msi(_path: &Path) -> Result<(), String> {
    Err("MSI install is only on Windows".into())
}

/// Filesystem operations the portable replace needs, so tests cover failure handling offline.
#[cfg(any(target_os = "windows", test))]
pub trait PortableOps {
    fn is_dir(&self, path: &Path) -> bool;
    fn is_file(&self, path: &Path) -> bool;
    fn exists(&self, path: &Path) -> bool;
    fn copy_tree(&self, from: &Path, to: &Path) -> Result<(), String>;
    fn move_path(&self, from: &Path, to: &Path) -> Result<(), String>;
    fn remove_tree(&self, path: &Path) -> Result<(), String>;
}

/// A portable update is only acceptable when it really is an application folder holding the
/// executable we are running. A checksummed zip can still be packed wrong, and swapping one in
/// would replace a working installation with a broken one.
#[cfg(any(target_os = "windows", test))]
pub fn validate_portable(ops: &dyn PortableOps, root: &Path, exe_name: &str) -> Result<(), String> {
    if exe_name.is_empty() {
        return Err("could not find the running executable name".into());
    }
    if !ops.is_dir(root) {
        return Err("the downloaded update is not an application folder".into());
    }
    if !ops.is_file(&root.join(exe_name)) {
        return Err("the downloaded update has no WordCraft executable".into());
    }
    Ok(())
}

/// Copy the validated download beside the installation (`staged`), so the swap that follows is a
/// rename on the same volume rather than a copy over the live files. A failed or partial copy is
/// removed and never touches the installation.
#[cfg(any(target_os = "windows", test))]
pub fn stage_portable(ops: &dyn PortableOps, app_root: &Path, staged: &Path, exe_name: &str) -> Result<(), String> {
    if let Err(e) = ops.copy_tree(app_root, staged) {
        let _ = ops.remove_tree(staged);
        return Err(e);
    }
    if let Err(e) = validate_portable(ops, staged, exe_name) {
        let _ = ops.remove_tree(staged);
        return Err(e);
    }
    Ok(())
}

/// Replace `dest` with the staged build. The previous installation is renamed aside before the new
/// one goes in and restored if that fails, so a failure never leaves a half-replaced folder. With
/// no previous installation the staged build simply takes its place.
#[cfg(any(target_os = "windows", test))]
pub fn replace_portable(ops: &dyn PortableOps, staged: &Path, dest: &Path, exe_name: &str) -> Result<(), String> {
    validate_portable(ops, staged, exe_name)?;
    let parent = dest.parent().ok_or("could not find the running install folder")?;
    if !ops.exists(dest) {
        return match ops.move_path(staged, dest) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = ops.remove_tree(staged);
                Err(e)
            }
        };
    }
    let backup = parent.join(format!(".wordcraft-previous-{}", random_hex(8)?));
    if let Err(e) = ops.move_path(dest, &backup) {
        return Err(format!("could not set the previous installation aside: {e}"));
    }
    if let Err(e) = ops.move_path(staged, dest) {
        // The new build is still staged: drop it, then put the previous installation back.
        let _ = ops.remove_tree(staged);
        let detail = match ops.move_path(&backup, dest) {
            Ok(()) => e,
            Err(restore) => format!("{e}; restoring the previous installation also failed: {restore}"),
        };
        return Err(detail);
    }
    let _ = ops.remove_tree(&backup);
    Ok(())
}

/// Arguments for the Windows portable launcher. Owned strings keep the array a single element type
/// (a mixed `&str`/`String` array does not compile on Windows).
#[cfg(any(target_os = "windows", test))]
fn portable_helper_args(script: &str, pid: u32, helper: &str, staged: &str, dest: &str, exe_name: &str, staging: &str) -> Vec<String> {
    vec![
        "-NoProfile".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        script.into(),
        "-WaitPid".into(),
        pid.to_string(),
        "-Helper".into(),
        helper.into(),
        "-Staged".into(),
        staged.into(),
        "-Dest".into(),
        dest.into(),
        "-ExeName".into(),
        exe_name.into(),
        "-Staging".into(),
        staging.into(),
    ]
}

/// The launcher waits for the running app to exit (freeing its files), starts the copied helper
/// that performs the swap in Rust, then removes the helper and the private staging whatever the
/// outcome. It is only a waiter: the replacement itself is [`replace_portable`].
#[cfg(any(target_os = "windows", test))]
fn portable_script() -> &'static str {
    r#"param([int]$WaitPid, [string]$Helper, [string]$Staged, [string]$Dest, [string]$ExeName, [string]$Staging)
$ErrorActionPreference = 'Stop'
Wait-Process -Id $WaitPid -ErrorAction SilentlyContinue
try { Start-Process -FilePath $Helper -ArgumentList ('--apply-portable-update "{0}" "{1}" "{2}"' -f $Staged, $Dest, $ExeName) -Wait } catch {}
Remove-Item -Force -LiteralPath $Helper -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force -LiteralPath $Staging -ErrorAction SilentlyContinue
"#
}

/// The application folder inside the extracted zip: the release wraps its files in one top-level
/// folder (`wordcraft-…-portable/`), but a flat zip is accepted too.
#[cfg(target_os = "windows")]
fn find_portable_root(extract: &Path, exe_name: &str) -> Option<PathBuf> {
    if extract.join(exe_name).is_file() {
        return Some(extract.to_path_buf());
    }
    let first = std::fs::read_dir(extract).ok()?.flatten().next()?;
    (first.file_type().ok()?.is_dir() && first.path().join(exe_name).is_file()).then(|| first.path())
}

// `powershell` only exists on Windows, where this path actually runs.
#[cfg(target_os = "windows")]
fn windows_portable(zip: &Path, exe: &str, staging: &Path) -> Result<(), String> {
    let extract = staging.join("extract");
    std::fs::create_dir(&extract).map_err(|e| e.to_string())?;
    let status = Command::new("tar")
        .args(["-xf", zip.to_str().ok_or("bad path")?, "-C", extract.to_str().ok_or("bad path")?])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Could not unpack the portable zip.".into());
    }
    let exe_name = Path::new(exe).file_name().and_then(|n| n.to_str()).ok_or("could not find the running executable name")?;
    let app_root = find_portable_root(&extract, exe_name).ok_or("the downloaded update has no WordCraft executable")?;
    let dest = Path::new(exe);
    let parent = dest.parent().ok_or("could not find the running install folder")?;
    // Stage on the installation's own volume, so the swap the helper performs is a rename.
    let staged = parent.join(format!(".wordcraft-update-{}", random_hex(8)?));
    let ops = RealPortableOps;
    stage_portable(&ops, &app_root, &staged, exe_name)?;
    // The helper must not run from `staged` (that folder becomes the installation): copy the new
    // build's own executable out of it first, so the folder being moved is never a running program.
    let helper = parent.join(format!(".wordcraft-helper-{}", random_hex(8)?)).with_extension("exe");
    match write_portable_launcher(&staged, &helper, staging, exe_name, dest) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&helper);
            let _ = std::fs::remove_dir_all(&staged);
            Err(e)
        }
    }
}

/// Copy the helper executable out of the staged build and start the launcher, which waits for this
/// process to exit before running the helper. On any failure the caller removes the helper and the
/// staged folder.
#[cfg(target_os = "windows")]
fn write_portable_launcher(staged: &Path, helper: &Path, staging: &Path, exe_name: &str, dest: &Path) -> Result<(), String> {
    std::fs::copy(staged.join(exe_name), helper).map_err(|e| e.to_string())?;
    let script = staging.join("apply-portable.ps1");
    std::fs::write(&script, portable_script()).map_err(|e| e.to_string())?;
    let args = portable_helper_args(
        script.to_str().ok_or("bad path")?,
        std::process::id(),
        helper.to_str().ok_or("bad path")?,
        staged.to_str().ok_or("bad path")?,
        dest.to_str().ok_or("bad path")?,
        exe_name,
        staging.to_str().ok_or("bad path")?,
    );
    let child = Command::new("powershell").args(&args).spawn().map_err(|e| e.to_string())?;
    if child.id() == 0 {
        return Err("Could not start the portable update helper.".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
struct RealPortableOps;

#[cfg(target_os = "windows")]
impl PortableOps for RealPortableOps {
    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
    fn copy_tree(&self, from: &Path, to: &Path) -> Result<(), String> {
        copy_dir_all(from, to)
    }
    fn move_path(&self, from: &Path, to: &Path) -> Result<(), String> {
        std::fs::rename(from, to).map_err(|e| e.to_string())
    }
    fn remove_tree(&self, path: &Path) -> Result<(), String> {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    }
}

/// When the process was started as the portable-update helper (`--apply-portable-update <staged>
/// <dest> <exe>`), apply the swap and relaunch, and return the exit code. `None` for a normal
/// start, so the interface runs.
#[cfg(target_os = "windows")]
pub fn run_portable_helper_from_args() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--apply-portable-update") {
        return None;
    }
    let rest: Vec<String> = args.collect();
    Some(if portable_helper_main(&rest).is_ok() { 0 } else { 1 })
}

#[cfg(not(target_os = "windows"))]
pub fn run_portable_helper_from_args() -> Option<i32> {
    None
}

#[cfg(target_os = "windows")]
fn portable_helper_main(args: &[String]) -> Result<(), String> {
    let [staged, dest, exe_name] = args else { return Err("bad portable-update arguments".into()) };
    let (staged, dest) = (Path::new(staged), Path::new(dest));
    let result = replace_portable(&RealPortableOps, staged, dest, exe_name);
    // Relaunch whatever is now installed: the restored previous build after a failure, the new one
    // after a success. A failed update must never leave the user without a running copy.
    let launch = dest.join(exe_name);
    if launch.is_file() {
        let _ = Command::new(&launch).spawn();
    }
    result
}

#[cfg(not(target_os = "windows"))]
fn windows_portable(_zip: &Path, _exe: &str, _staging: &Path) -> Result<(), String> {
    Err("portable zip update is only on Windows".into())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_deb(path: &Path) -> Result<(), String> {
    let p = path.to_str().ok_or("bad path")?;
    let status = Command::new("pkexec").args(["dpkg", "-i", p]).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err("dpkg install was cancelled or failed.".into()) }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn linux_deb(_path: &Path) -> Result<(), String> {
    Err(".deb install is only on Linux".into())
}

/// The `.app` bundle that contains `exe`, so an update replaces the installation the user is
/// actually running (from `/Applications`, `~/Applications` or anywhere else), never a different one.
#[cfg(any(target_os = "macos", test))]
pub fn app_bundle_for(exe: &str) -> Option<PathBuf> {
    let mut cur = Path::new(exe).parent();
    while let Some(dir) = cur {
        if dir.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")) {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

/// Filesystem operations the macOS bundle swap needs, so tests cover failure handling offline.
#[cfg(any(target_os = "macos", test))]
pub trait BundleOps {
    fn is_dir(&self, path: &Path) -> bool;
    fn is_file(&self, path: &Path) -> bool;
    /// True when `path` is a regular file with an executable bit set.
    fn is_executable(&self, path: &Path) -> bool;
    fn exists(&self, path: &Path) -> bool;
    fn copy_tree(&self, from: &Path, to: &Path) -> Result<(), String>;
    fn move_path(&self, from: &Path, to: &Path) -> Result<(), String>;
    fn remove_tree(&self, path: &Path) -> Result<(), String>;
}

/// A `.app` is only acceptable when it is a real bundle: `Contents/`, the `Info.plist` metadata and
/// the executable we are currently running. A checksummed DMG can still hold a truncated or
/// otherwise broken bundle, and swapping one in would replace a working installation.
#[cfg(any(target_os = "macos", test))]
pub fn validate_bundle(ops: &dyn BundleOps, root: &Path, exe_name: &str) -> Result<(), String> {
    if !ops.is_dir(root) {
        return Err("the downloaded update is not an application bundle".into());
    }
    if !ops.is_dir(&root.join("Contents")) {
        return Err("the downloaded application bundle is incomplete".into());
    }
    if !ops.is_file(&root.join("Contents/Info.plist")) {
        return Err("the downloaded application bundle is missing its Info.plist".into());
    }
    if !ops.is_dir(&root.join("Contents/MacOS")) {
        return Err("the downloaded application bundle is missing its executable folder".into());
    }
    if exe_name.is_empty() {
        return Err("could not find the running executable name".into());
    }
    let exe = root.join("Contents/MacOS").join(exe_name);
    if !ops.is_file(&exe) || !ops.is_executable(&exe) {
        return Err("the downloaded application bundle has no runnable executable".into());
    }
    Ok(())
}

/// Copy `staged` beside `dest`, validate the copy, then swap it in. The previous bundle is kept
/// until a complete new one is in place, so a failed copy, an incomplete bundle or a failed move
/// leaves the installation exactly as it was.
#[cfg(any(target_os = "macos", test))]
pub fn replace_bundle(ops: &dyn BundleOps, staged: &Path, dest: &Path, exe_name: &str) -> Result<(), String> {
    let parent = dest.parent().ok_or("could not find the install folder")?;
    if !ops.is_dir(staged) {
        return Err("the downloaded update is not an application bundle".into());
    }
    let tmp = parent.join(format!(".wordcraft-update-{}", random_hex(8)?));
    if let Err(e) = ops.copy_tree(staged, &tmp) {
        let _ = ops.remove_tree(&tmp);
        return Err(e);
    }
    if let Err(e) = validate_bundle(ops, &tmp, exe_name) {
        let _ = ops.remove_tree(&tmp);
        return Err(e);
    }
    if ops.exists(dest) {
        let backup = parent.join(format!(".wordcraft-previous-{}", random_hex(8)?));
        if let Err(e) = ops.move_path(dest, &backup) {
            let _ = ops.remove_tree(&tmp);
            return Err(e);
        }
        if let Err(e) = ops.move_path(&tmp, dest) {
            // Put the previous installation back before giving up.
            let _ = ops.move_path(&backup, dest);
            let _ = ops.remove_tree(&tmp);
            return Err(e);
        }
        let _ = ops.remove_tree(&backup);
    } else if let Err(e) = ops.move_path(&tmp, dest) {
        let _ = ops.remove_tree(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
struct RealBundleOps;

#[cfg(target_os = "macos")]
impl BundleOps for RealBundleOps {
    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
    fn is_executable(&self, path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(m) => m.is_file() && m.permissions().mode() & 0o111 != 0,
            Err(_) => false,
        }
    }
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
    fn copy_tree(&self, from: &Path, to: &Path) -> Result<(), String> {
        copy_dir_all(from, to)
    }
    fn move_path(&self, from: &Path, to: &Path) -> Result<(), String> {
        std::fs::rename(from, to).map_err(|e| e.to_string())
    }
    fn remove_tree(&self, path: &Path) -> Result<(), String> {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn copy_dir_all(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else if ty.is_symlink() {
            #[cfg(unix)]
            {
                let link = std::fs::read_link(entry.path()).map_err(|e| e.to_string())?;
                std::os::unix::fs::symlink(link, &target).map_err(|e| e.to_string())?;
            }
            #[cfg(not(unix))]
            {
                std::fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
            }
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn mac_dmg(dmg: &Path, staging: &Path, exe: &str) -> Result<(), String> {
    let dest = app_bundle_for(exe).ok_or("WordCraft is not running from an application bundle")?;
    let mount = staging.join("mnt");
    std::fs::create_dir(&mount).map_err(|e| e.to_string())?;
    let dmg_s = dmg.to_str().ok_or("bad path")?;
    let mount_s = mount.to_str().ok_or("bad path")?;
    let attach =
        Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-mountpoint", mount_s, dmg_s]).status().map_err(|e| e.to_string())?;
    if !attach.success() {
        return Err("Could not open the disk image.".into());
    }
    let app_src = mount.join("WordCraft.app");
    let exe_name = Path::new(exe).file_name().and_then(|n| n.to_str()).unwrap_or("");
    let result = if !app_src.is_dir() {
        Err("The disk image does not contain WordCraft.app.".into())
    } else {
        replace_bundle(&RealBundleOps, &app_src, &dest, exe_name)
    };
    // Detaching a read-only image never touches what was already copied; a failed detach must not
    // mask a successful (or failed) install.
    let _ = Command::new("hdiutil").args(["detach", mount_s]).status();
    result
}

#[cfg(not(target_os = "macos"))]
fn mac_dmg(_dmg: &Path, _staging: &Path, _exe: &str) -> Result<(), String> {
    Err("disk image install is only on macOS".into())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn exclusive_file_refuses_existing() {
        let dir = private_staging_dir().unwrap();
        write_exclusive_file(&dir, "pkg.deb", b"one").unwrap();
        assert!(write_exclusive_file(&dir, "pkg.deb", b"two").is_err());
    }

    #[test]
    fn staging_dirs_are_unique() {
        let a = private_staging_dir().unwrap();
        let b = private_staging_dir().unwrap();
        assert_ne!(a, b);
    }

    #[cfg(unix)]
    #[test]
    fn staging_dir_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = private_staging_dir().unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "staging must be private");
    }

    #[test]
    fn windows_helper_args_carry_owned_scripts_and_pid() {
        let args = portable_helper_args("s.ps1", 42, "h.exe", "/upd", "/dest", "wordcraft.exe", "/stage");
        assert_eq!(args.first().map(String::as_str), Some("-NoProfile"));
        let after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str);
        assert_eq!(after("-File"), Some("s.ps1"));
        assert_eq!(after("-WaitPid"), Some("42"));
        assert_eq!(after("-Helper"), Some("h.exe"));
        assert_eq!(after("-Staged"), Some("/upd"));
        assert_eq!(after("-Dest"), Some("/dest"));
        assert_eq!(after("-ExeName"), Some("wordcraft.exe"));
        assert_eq!(after("-Staging"), Some("/stage"));
    }

    #[test]
    fn portable_script_restores_and_cleans_up() {
        let s = portable_script();
        assert!(s.contains("Wait-Process"), "the launcher waits for the app to exit");
        assert!(s.contains("--apply-portable-update"), "it runs the Rust helper");
        assert!(s.contains("Remove-Item -Force -LiteralPath $Helper"), "the helper is removed");
        assert!(s.contains("Remove-Item -Recurse -Force -LiteralPath $Staging"), "staging is removed");
    }

    #[test]
    fn app_bundle_comes_from_the_running_executable() {
        assert_eq!(app_bundle_for("/Applications/WordCraft.app/Contents/MacOS/wordcraft"), Some(PathBuf::from("/Applications/WordCraft.app")));
        assert_eq!(
            app_bundle_for("/Users/me/Applications/WordCraft.app/Contents/MacOS/wordcraft"),
            Some(PathBuf::from("/Users/me/Applications/WordCraft.app")),
            "a copy in another folder updates in place"
        );
        assert_eq!(app_bundle_for("/usr/local/bin/wordcraft"), None);
        assert_eq!(app_bundle_for(""), None);
    }

    /// A fake bundle filesystem: records moves and can fail copying, produce an incomplete bundle
    /// or fail the final swap.
    #[derive(Default)]
    struct FakeOps {
        dirs: RefCell<HashSet<PathBuf>>,
        files: RefCell<HashSet<PathBuf>>,
        execs: RefCell<HashSet<PathBuf>>,
        moves: RefCell<Vec<(PathBuf, PathBuf)>>,
        fail_copy: bool,
        /// A copied bundle whose `Contents/` is present but has no `Info.plist`/executable.
        empty_contents: bool,
        /// A copied bundle with metadata and a `MacOS/` folder but no runnable executable.
        no_executable: bool,
        fail_swap: bool,
    }

    impl FakeOps {
        fn with_dir(self, path: &str) -> Self {
            self.dirs.borrow_mut().insert(PathBuf::from(path));
            self
        }
    }

    impl BundleOps for FakeOps {
        fn is_dir(&self, path: &Path) -> bool {
            self.dirs.borrow().contains(path)
        }
        fn is_file(&self, path: &Path) -> bool {
            self.files.borrow().contains(path)
        }
        fn is_executable(&self, path: &Path) -> bool {
            self.execs.borrow().contains(path)
        }
        fn exists(&self, path: &Path) -> bool {
            self.dirs.borrow().contains(path) || self.files.borrow().contains(path)
        }
        fn copy_tree(&self, _from: &Path, to: &Path) -> Result<(), String> {
            if self.fail_copy {
                return Err("copy failed".into());
            }
            self.dirs.borrow_mut().insert(to.to_path_buf());
            self.dirs.borrow_mut().insert(to.join("Contents"));
            if !self.empty_contents {
                self.files.borrow_mut().insert(to.join("Contents/Info.plist"));
                self.dirs.borrow_mut().insert(to.join("Contents/MacOS"));
                if !self.no_executable {
                    self.files.borrow_mut().insert(to.join("Contents/MacOS/wordcraft"));
                    self.execs.borrow_mut().insert(to.join("Contents/MacOS/wordcraft"));
                }
            }
            Ok(())
        }
        fn move_path(&self, from: &Path, to: &Path) -> Result<(), String> {
            self.moves.borrow_mut().push((from.to_path_buf(), to.to_path_buf()));
            if self.fail_swap && from.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".wordcraft-update-")) {
                return Err("swap failed".into());
            }
            self.retain_not_under(from);
            self.dirs.borrow_mut().insert(to.to_path_buf());
            Ok(())
        }
        fn remove_tree(&self, path: &Path) -> Result<(), String> {
            self.retain_not_under(path);
            Ok(())
        }
    }

    impl FakeOps {
        /// Drop every entry at or below `root` (a real move/remove follows the whole tree).
        fn retain_not_under(&self, root: &Path) {
            let keep = |p: &PathBuf| p != root && !p.starts_with(root);
            self.dirs.borrow_mut().retain(keep);
            self.files.borrow_mut().retain(|p| keep(p));
            self.execs.borrow_mut().retain(|p| keep(p));
        }
    }

    fn staged() -> PathBuf {
        PathBuf::from("/stage/WordCraft.app")
    }

    fn dest() -> PathBuf {
        PathBuf::from("/apps/WordCraft.app")
    }

    fn run_replace(ops: &FakeOps) -> Result<(), String> {
        replace_bundle(ops, &staged(), &dest(), "wordcraft")
    }

    #[test]
    fn replace_bundle_swaps_and_clears_the_backup() {
        let ops = FakeOps::default().with_dir("/stage/WordCraft.app").with_dir("/apps/WordCraft.app");
        run_replace(&ops).unwrap();
        assert!(ops.is_dir(&dest()), "new bundle is in place");
        assert!(
            !ops.dirs.borrow().iter().chain(ops.files.borrow().iter()).any(|p| p.to_string_lossy().contains(".wordcraft-previous-")),
            "backup removed"
        );
        assert!(
            !ops.dirs.borrow().iter().chain(ops.files.borrow().iter()).any(|p| p.to_string_lossy().contains(".wordcraft-update-")),
            "staging removed"
        );
    }

    #[test]
    fn replace_bundle_keeps_install_when_copy_fails() {
        let ops = FakeOps { fail_copy: true, ..FakeOps::default() }.with_dir("/stage/WordCraft.app").with_dir("/apps/WordCraft.app");
        assert!(run_replace(&ops).is_err());
        assert!(ops.is_dir(&dest()), "the installed bundle is untouched");
    }

    /// The regression for #15: `Contents/` exists but the bundle is broken (no Info.plist and no
    /// executable). It must not replace the working installation.
    #[test]
    fn replace_bundle_rejects_a_present_but_incomplete_bundle() {
        let ops = FakeOps { empty_contents: true, ..FakeOps::default() }.with_dir("/stage/WordCraft.app").with_dir("/apps/WordCraft.app");
        let err = run_replace(&ops).unwrap_err();
        assert!(err.contains("incomplete") || err.contains("Info.plist") || err.contains("executable"), "{err}");
        assert!(ops.is_dir(&dest()), "the installed bundle is untouched");
        assert!(
            !ops.dirs.borrow().iter().chain(ops.files.borrow().iter()).any(|p| p.to_string_lossy().contains(".wordcraft-update-")),
            "staging removed"
        );
    }

    #[test]
    fn replace_bundle_rejects_a_bundle_without_a_runnable_executable() {
        // Metadata and `Contents/MacOS/` are present, but the executable file is missing.
        let ops = FakeOps { no_executable: true, ..FakeOps::default() }.with_dir("/stage/WordCraft.app").with_dir("/apps/WordCraft.app");
        let err = run_replace(&ops).unwrap_err();
        assert!(err.contains("runnable executable"), "{err}");
        assert!(ops.is_dir(&dest()), "the installed bundle is untouched");
    }

    #[test]
    fn replace_bundle_restores_previous_on_swap_failure() {
        let ops = FakeOps { fail_swap: true, ..FakeOps::default() }.with_dir("/stage/WordCraft.app").with_dir("/apps/WordCraft.app");
        assert!(run_replace(&ops).is_err());
        assert!(ops.is_dir(&dest()), "the previous bundle is restored");
        assert!(
            !ops.dirs.borrow().iter().chain(ops.files.borrow().iter()).any(|p| p.to_string_lossy().contains(".wordcraft-update-")),
            "staging removed"
        );
    }

    #[test]
    fn replace_bundle_creates_a_fresh_install() {
        let ops = FakeOps::default().with_dir("/stage/WordCraft.app");
        run_replace(&ops).unwrap();
        assert!(ops.is_dir(&dest()), "installed where none existed");
    }

    #[test]
    fn replace_bundle_refuses_a_non_bundle() {
        let ops = FakeOps::default().with_dir("/apps/WordCraft.app");
        assert!(run_replace(&ops).is_err());
        assert!(ops.is_dir(&dest()), "the installed bundle is untouched");
    }

    // ---- Windows portable replace (#15, review P1) --------------------------------------------

    const PEXE: &str = "wordcraft.exe";

    fn pstaged() -> PathBuf {
        PathBuf::from("/apps/.wordcraft-update-test")
    }

    fn pdest() -> PathBuf {
        PathBuf::from("/apps/wordcraft")
    }

    /// A fake filesystem with the staged build (`staged`, holding the executable) and, when
    /// `installed`, an existing installation at [`pdest`].
    fn portable(staged: &Path, installed: bool) -> FakePortable {
        let mut ops = FakePortable::default().with_dir(staged).with_file(staged.join(PEXE));
        if installed {
            ops = ops.with_dir(pdest()).with_file(pdest().join(PEXE));
        }
        ops
    }

    /// A fake filesystem for the portable replace, recording moves and able to fail parts of it.
    #[derive(Default)]
    struct FakePortable {
        dirs: RefCell<HashSet<PathBuf>>,
        files: RefCell<HashSet<PathBuf>>,
        moves: RefCell<Vec<(PathBuf, PathBuf)>>,
        /// `copy_tree` writes a partial tree, then fails.
        partial_copy: bool,
        /// The staged build cannot be moved into place.
        fail_swap: bool,
        /// The previous installation cannot be renamed aside.
        fail_aside: bool,
    }

    impl FakePortable {
        fn with_dir(self, path: impl AsRef<Path>) -> Self {
            self.dirs.borrow_mut().insert(path.as_ref().to_path_buf());
            self
        }
        fn with_file(self, path: impl AsRef<Path>) -> Self {
            self.files.borrow_mut().insert(path.as_ref().to_path_buf());
            self
        }
        fn retain_not_under(&self, root: &Path) {
            let keep = |p: &PathBuf| p != root && !p.starts_with(root);
            self.dirs.borrow_mut().retain(keep);
            self.files.borrow_mut().retain(|p| keep(p));
        }
        /// Any leftover staging or backup folder.
        fn has_artifact(&self) -> bool {
            self.dirs
                .borrow()
                .iter()
                .chain(self.files.borrow().iter())
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
                .any(|n| n.starts_with(".wordcraft-update-") || n.starts_with(".wordcraft-previous-"))
        }
    }

    impl PortableOps for FakePortable {
        fn is_dir(&self, path: &Path) -> bool {
            self.dirs.borrow().contains(path)
        }
        fn is_file(&self, path: &Path) -> bool {
            self.files.borrow().contains(path)
        }
        fn exists(&self, path: &Path) -> bool {
            self.dirs.borrow().contains(path) || self.files.borrow().contains(path)
        }
        fn copy_tree(&self, from: &Path, to: &Path) -> Result<(), String> {
            if !self.is_dir(from) {
                return Err("source folder is missing".into());
            }
            if self.partial_copy {
                // Some files landed before the failure; they must be cleaned up, never installed.
                self.dirs.borrow_mut().insert(to.to_path_buf());
                self.files.borrow_mut().insert(to.join(format!("{PEXE}.partial")));
                return Err("copy failed partway".into());
            }
            let rewrite = |p: &PathBuf| p.strip_prefix(from).ok().map(|rest| to.join(rest));
            let dirs: Vec<PathBuf> = self.dirs.borrow().iter().filter_map(rewrite).collect();
            let files: Vec<PathBuf> = self.files.borrow().iter().filter_map(rewrite).collect();
            self.dirs.borrow_mut().extend(dirs);
            self.files.borrow_mut().extend(files);
            self.dirs.borrow_mut().insert(to.to_path_buf());
            Ok(())
        }
        fn move_path(&self, from: &Path, to: &Path) -> Result<(), String> {
            self.moves.borrow_mut().push((from.to_path_buf(), to.to_path_buf()));
            if self.fail_aside && to.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".wordcraft-previous-")) {
                return Err("could not rename the previous installation".into());
            }
            if self.fail_swap && from.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".wordcraft-update-")) {
                return Err("could not rename the new build into place".into());
            }
            let rewrite = |p: &PathBuf| p.strip_prefix(from).ok().map(|rest| to.join(rest));
            let dirs: Vec<PathBuf> = self.dirs.borrow().iter().filter_map(rewrite).collect();
            let files: Vec<PathBuf> = self.files.borrow().iter().filter_map(rewrite).collect();
            self.retain_not_under(from);
            self.dirs.borrow_mut().extend(dirs);
            self.files.borrow_mut().extend(files);
            self.dirs.borrow_mut().insert(to.to_path_buf());
            Ok(())
        }
        fn remove_tree(&self, path: &Path) -> Result<(), String> {
            self.retain_not_under(path);
            Ok(())
        }
    }

    #[test]
    fn portable_validation_requires_the_executable() {
        let ops = FakePortable::default().with_dir("/extract");
        assert!(validate_portable(&ops, Path::new("/extract"), PEXE).is_err(), "no executable");
        let ops = portable(&pstaged(), false);
        assert!(validate_portable(&ops, &pstaged(), PEXE).is_ok());
        assert!(validate_portable(&ops, &pstaged(), "").is_err(), "empty exe name");
        assert!(validate_portable(&ops, Path::new("/nope"), PEXE).is_err(), "not a folder");
    }

    /// A staging copy that fails partway must leave the installation untouched and no partial copy
    /// behind.
    #[test]
    fn portable_staging_partial_copy_cleans_up_and_keeps_the_install() {
        let ops = FakePortable { partial_copy: true, ..portable(&pstaged(), true) }.with_dir("/extract").with_file(Path::new("/extract").join(PEXE));
        let err = stage_portable(&ops, Path::new("/extract"), &pstaged(), PEXE).unwrap_err();
        assert!(err.contains("partway") || err.contains("copy"), "{err}");
        assert!(ops.is_dir(&pdest()), "the installed app is untouched");
        assert!(ops.is_file(&pdest().join(PEXE)));
        assert!(!ops.has_artifact(), "no partial staging left");
    }

    #[test]
    fn portable_replace_swaps_and_clears_backup() {
        let ops = portable(&pstaged(), true);
        replace_portable(&ops, &pstaged(), &pdest(), PEXE).unwrap();
        assert!(ops.is_dir(&pdest()), "the new build is in place");
        assert!(ops.is_file(&pdest().join(PEXE)), "with its executable");
        assert!(!ops.has_artifact(), "staging and backup removed");
    }

    /// The regression for #15: when the new build cannot be moved into place after the previous
    /// installation was set aside, the previous one must come back and nothing partial may remain.
    #[test]
    fn portable_replace_restores_previous_when_swap_fails() {
        let ops = FakePortable { fail_swap: true, ..portable(&pstaged(), true) };
        let err = replace_portable(&ops, &pstaged(), &pdest(), PEXE).unwrap_err();
        assert!(err.contains("into place"), "{err}");
        assert!(ops.is_dir(&pdest()), "the previous installation is restored");
        assert!(ops.is_file(&pdest().join(PEXE)), "with its executable");
        assert!(!ops.has_artifact(), "no staging or backup left");
    }

    #[test]
    fn portable_replace_keeps_the_install_when_it_cannot_be_set_aside() {
        let ops = FakePortable { fail_aside: true, ..portable(&pstaged(), true) };
        assert!(replace_portable(&ops, &pstaged(), &pdest(), PEXE).is_err());
        assert!(ops.is_dir(&pdest()), "the installation is untouched");
    }

    #[test]
    fn portable_replace_refuses_a_staged_folder_without_the_executable() {
        let ops = FakePortable::default().with_dir(pstaged()).with_dir(pdest());
        let err = replace_portable(&ops, &pstaged(), &pdest(), PEXE).unwrap_err();
        assert!(err.contains("executable"), "{err}");
        assert!(ops.is_dir(&pdest()), "the installation is untouched");
    }

    #[test]
    fn portable_replace_creates_a_fresh_install() {
        let ops = portable(&pstaged(), false);
        replace_portable(&ops, &pstaged(), &pdest(), PEXE).unwrap();
        assert!(ops.is_dir(&pdest()), "installed where none existed");
    }
}

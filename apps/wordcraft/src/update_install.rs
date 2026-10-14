//! One-click update handoff after a verified download (#15).
//!
//! The platform installers run only after `file.checkForUpdates` verified the download against the
//! release's published SHA-256 checksums. Each path is arranged so a failure never leaves a
//! half-installed application: the macOS bundle is staged, validated and swapped with the old
//! bundle kept until the new one is in place. Portable zip installs are not replaced in place; they
//! use the release page instead.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use wordcraft_engine::update::{InstallFn, InstallHandoff, InstallKind, InstallOutcome};

pub fn handoff() -> InstallFn {
    std::sync::Arc::new(run)
}

fn run(job: InstallHandoff) -> Result<InstallOutcome, String> {
    let staging = private_staging_dir()?;
    let result = install_into(&job, &staging);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn install_into(job: &InstallHandoff, staging: &Path) -> Result<InstallOutcome, String> {
    let file_name = safe_name(&job.file_name);
    let path = write_exclusive_file(staging, &file_name, &job.bytes)?;
    match job.kind {
        InstallKind::WindowsMsi => windows_msi(&path),
        InstallKind::LinuxDeb => linux_deb(&path),
        InstallKind::MacDmg | InstallKind::MacApp => mac_dmg(&path, staging, &job.exe_path),
        InstallKind::WindowsPortableZip | InstallKind::Flatpak | InstallKind::AppImage | InstallKind::Unknown => {
            Err("This install type must use the release page.".into())
        }
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

fn wait_child(mut child: std::process::Child, secs: u64) -> Result<std::process::ExitStatus, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    return Err("the installer took too long".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
const INSTALLER_WAIT_SECS: u64 = 60 * 60;

/// Documented Windows Installer return codes we act on: `0` is success, `3010` is success with a
/// restart required to finish, `1641` is success where the installer restarted the machine, and
/// `1602` is the user cancelling.
#[cfg(any(target_os = "windows", test))]
pub const MSI_SUCCESS: u32 = 0;
#[cfg(any(target_os = "windows", test))]
pub const MSI_SUCCESS_REBOOT_REQUIRED: u32 = 3010;
#[cfg(any(target_os = "windows", test))]
pub const MSI_SUCCESS_REBOOT_INITIATED: u32 = 1641;
#[cfg(any(target_os = "windows", test))]
pub const MSI_CANCELLED: u32 = 1602;

/// What an MSI run produced, independent of the host so tests need no Windows.
#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsiOutcome {
    Installed,
    /// Installed, but a restart is needed to finish replacing in-use files.
    InstalledRebootRequired,
    Cancelled,
    Failed(u32),
}

/// Classify a documented MSI return code (Microsoft's installer error codes).
#[cfg(any(target_os = "windows", test))]
pub fn msi_outcome(code: u32) -> MsiOutcome {
    match code {
        MSI_SUCCESS => MsiOutcome::Installed,
        MSI_SUCCESS_REBOOT_REQUIRED | MSI_SUCCESS_REBOOT_INITIATED => MsiOutcome::InstalledRebootRequired,
        MSI_CANCELLED => MsiOutcome::Cancelled,
        other => MsiOutcome::Failed(other),
    }
}

/// The `msiexec` arguments for an unattended in-place upgrade.
///
/// `/passive` installs with a progress bar but no prompts. `/norestart` suppresses any restart the
/// package would otherwise trigger: Microsoft documents that `/passive` alone can restart the
/// computer without prompting, which would close WordCraft before it could prompt to save.
/// `MSIRESTARTMANAGERCONTROL=DisableShutdown` stops the Restart Manager from shutting running
/// applications down to replace their files, so no installer-triggered close can bypass the save
/// prompt; in-use files are instead scheduled for the restart (reported as `3010`).
#[cfg(any(target_os = "windows", test))]
pub fn msi_args(path: &str) -> Vec<String> {
    vec!["/i".into(), path.into(), "/passive".into(), "/norestart".into(), "MSIRESTARTMANAGERCONTROL=DisableShutdown".into()]
}

/// Runs `msiexec` and returns its raw exit code, so tests cover the arguments and return-code
/// handling offline.
#[cfg(any(target_os = "windows", test))]
pub trait MsiRunner {
    fn run(&self, args: &[String], timeout_secs: u64) -> Result<u32, String>;
}

#[cfg(any(target_os = "windows", test))]
const MSI_WAIT_SECS: u64 = 60 * 60;

/// Run the installer through `runner`, classifying its return code. A restart-required success is
/// reported (not an error), so the app can still quit cleanly and tell the user.
#[cfg(any(target_os = "windows", test))]
fn windows_msi_with(runner: &dyn MsiRunner, path: &Path) -> Result<InstallOutcome, String> {
    let p = path.to_str().ok_or("bad path")?;
    let code = runner.run(&msi_args(p), MSI_WAIT_SECS)?;
    match msi_outcome(code) {
        MsiOutcome::Installed => Ok(InstallOutcome::default()),
        MsiOutcome::InstalledRebootRequired => {
            log::info!("WordCraft was updated; restart the computer to finish (MSI code {code})");
            Ok(InstallOutcome { reboot_required: true })
        }
        MsiOutcome::Cancelled => Err("The update was cancelled.".into()),
        MsiOutcome::Failed(code) => Err(format!("msiexec exited with code {code}")),
    }
}

#[cfg(target_os = "windows")]
struct RealMsiRunner;

#[cfg(target_os = "windows")]
impl MsiRunner for RealMsiRunner {
    fn run(&self, args: &[String], timeout_secs: u64) -> Result<u32, String> {
        let child = Command::new("msiexec").args(args).spawn().map_err(|e| e.to_string())?;
        let status = wait_child(child, timeout_secs)?;
        status.code().map(|c| c as u32).ok_or_else(|| "msiexec was terminated by a signal".to_string())
    }
}

#[cfg(target_os = "windows")]
fn windows_msi(path: &Path) -> Result<InstallOutcome, String> {
    windows_msi_with(&RealMsiRunner, path)
}

#[cfg(not(target_os = "windows"))]
fn windows_msi(_path: &Path) -> Result<InstallOutcome, String> {
    Err("MSI install is only on Windows".into())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_deb(path: &Path) -> Result<InstallOutcome, String> {
    let p = path.to_str().ok_or("bad path")?;
    let child = Command::new("pkexec").args(["dpkg", "-i", p]).spawn().map_err(|e| e.to_string())?;
    let status = wait_child(child, INSTALLER_WAIT_SECS)?;
    if status.success() { Ok(InstallOutcome::default()) } else { Err("dpkg install was cancelled or failed.".into()) }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn linux_deb(_path: &Path) -> Result<InstallOutcome, String> {
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
    /// True when this user may create and remove files in `path` (a package directory such as
    /// `/Applications`). A folder the user cannot write is replaced through authorization instead.
    fn is_writable_dir(&self, path: &Path) -> bool;
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

/// A bundle replacement that needs OS authorization because the destination's folder is not
/// writable by this user (typically `/Applications`, owned by root). The real implementation asks
/// for administrator approval through `osascript`; tests inject a fake so a cancelled or refused
/// authorization is covered offline and an unwritable install never fails silently.
#[cfg(any(target_os = "macos", test))]
pub trait AuthorizedBundle {
    /// Replace `dest` with the validated bundle at `staged`, authenticating if needed.
    fn replace(&self, staged: &Path, dest: &Path, exe_name: &str) -> Result<(), String>;
}

/// Replace `dest` with the bundle at `staged` — directly when this user can write the folder, or
/// through [`AuthorizedBundle`] when the folder (such as `/Applications`) is not writable. The
/// staged bundle is validated either way, so a broken download is refused before anything moves.
#[cfg(any(target_os = "macos", test))]
pub fn install_bundle(ops: &dyn BundleOps, auth: &dyn AuthorizedBundle, staged: &Path, dest: &Path, exe_name: &str) -> Result<(), String> {
    let parent = dest.parent().ok_or("could not find the install folder")?;
    if ops.is_writable_dir(parent) {
        replace_bundle(ops, staged, dest, exe_name)
    } else {
        validate_bundle(ops, staged, exe_name)?;
        auth.replace(staged, dest, exe_name)
    }
}

/// `sh` single-quote quoting: `'` becomes `'\''`, everything else stays literal. The authorized
/// script is assembled only from paths we built, but a path can still contain a quote.
#[cfg(any(target_os = "macos", test))]
pub fn sh_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// The AppleScript `do shell script` argument: a string literal with `\` and `"` escaped. Without
/// this, a path containing a double quote would let the shell script escape the string literal.
#[cfg(any(target_os = "macos", test))]
pub fn apple_quote(script: &str) -> String {
    let mut out = String::with_capacity(script.len() + 2);
    out.push('"');
    for c in script.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The shell script that runs as root (through `do shell script`) to swap the bundle. It stages
/// the new bundle beside the old one, moves the old aside, swaps the new in and only then removes
/// the old; if the swap fails the old bundle is put back, so the install is never left missing.
#[cfg(any(target_os = "macos", test))]
pub fn authorized_replace_script(staged: &Path, dest: &Path) -> String {
    let staged = sh_quote(&staged.to_string_lossy());
    let dest = sh_quote(&dest.to_string_lossy());
    format!(
        "set -e\n\
         stage={staged}\n\
         dest={dest}\n\
         old=\"$dest.wordcraft-previous\"\n\
         tmp=\"$dest.wordcraft-update\"\n\
         rm -rf \"$old\" \"$tmp\"\n\
         /usr/bin/ditto \"$stage\" \"$tmp\"\n\
         if [ -e \"$dest\" ]; then\n\
           mv \"$dest\" \"$old\"\n\
           if mv \"$tmp\" \"$dest\"; then rm -rf \"$old\"; else mv \"$old\" \"$dest\"; exit 1; fi\n\
         else\n\
           mv \"$tmp\" \"$dest\"\n\
         fi\n"
    )
}

#[cfg(target_os = "macos")]
const AUTHORIZE_WAIT_SECS: u64 = 5 * 60;

#[cfg(target_os = "macos")]
struct RealAuthorizedBundle;

#[cfg(target_os = "macos")]
impl AuthorizedBundle for RealAuthorizedBundle {
    fn replace(&self, staged: &Path, dest: &Path, exe_name: &str) -> Result<(), String> {
        let _ = exe_name;
        let script = authorized_replace_script(staged, dest);
        let arg = format!("do shell script {} with administrator privileges", apple_quote(&script));
        let child = Command::new("osascript").args(["-e", &arg]).spawn().map_err(|e| e.to_string())?;
        let status = wait_child(child, AUTHORIZE_WAIT_SECS)?;
        if status.success() { Ok(()) } else { Err("The update needs your permission to replace WordCraft in /Applications.".into()) }
    }
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
    fn is_writable_dir(&self, path: &Path) -> bool {
        if !path.is_dir() {
            return false;
        }
        let probe = path.join(format!(".wordcraft-write-probe-{}", std::process::id()));
        match std::fs::File::create(&probe) {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                true
            }
            Err(_) => false,
        }
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

#[cfg(target_os = "macos")]
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
const DMG_COMMAND_WAIT_SECS: u64 = 10 * 60;

#[cfg(target_os = "macos")]
trait DmgRunner {
    fn attach(&self, dmg: &str, mount: &str) -> Result<(), String>;
    fn detach(&self, mount: &str);
}

#[cfg(target_os = "macos")]
struct RealDmgRunner;

#[cfg(target_os = "macos")]
impl DmgRunner for RealDmgRunner {
    fn attach(&self, dmg: &str, mount: &str) -> Result<(), String> {
        let mut cmd = Command::new("hdiutil");
        cmd.args(["attach", "-nobrowse", "-readonly", "-mountpoint", mount, dmg]);
        let status = run_command_status(cmd, DMG_COMMAND_WAIT_SECS)?;
        if status.success() { Ok(()) } else { Err("Could not open the disk image.".into()) }
    }
    fn detach(&self, mount: &str) {
        let mut cmd = Command::new("hdiutil");
        cmd.args(["detach", mount]);
        let _ = run_command_status(cmd, DMG_COMMAND_WAIT_SECS);
    }
}

#[cfg(target_os = "macos")]
fn run_command_status(mut cmd: Command, secs: u64) -> Result<std::process::ExitStatus, String> {
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    wait_child(child, secs)
}

#[cfg(target_os = "macos")]
fn mac_dmg_with(dmg: &Path, staging: &Path, exe: &str, runner: &dyn DmgRunner, auth: &dyn AuthorizedBundle) -> Result<(), String> {
    let dest = app_bundle_for(exe).ok_or("WordCraft is not running from an application bundle")?;
    let dest_lower = dest.to_string_lossy().to_ascii_lowercase();
    if !dest_lower.starts_with("/applications/") {
        return Err("One-click install only replaces WordCraft in /Applications.".into());
    }
    let mount = staging.join("mnt");
    std::fs::create_dir(&mount).map_err(|e| e.to_string())?;
    let dmg_s = dmg.to_str().ok_or("bad path")?;
    let mount_s = mount.to_str().ok_or("bad path")?;
    runner.attach(dmg_s, mount_s)?;
    let app_src = mount.join("WordCraft.app");
    let exe_name = Path::new(exe).file_name().and_then(|n| n.to_str()).unwrap_or("");
    let result = if !app_src.is_dir() {
        Err("The disk image does not contain WordCraft.app.".into())
    } else {
        install_bundle(&RealBundleOps, auth, &app_src, &dest, exe_name)
    };
    runner.detach(mount_s);
    result
}

#[cfg(target_os = "macos")]
fn mac_dmg(dmg: &Path, staging: &Path, exe: &str) -> Result<InstallOutcome, String> {
    mac_dmg_with(dmg, staging, exe, &RealDmgRunner, &RealAuthorizedBundle).map(|()| InstallOutcome::default())
}

#[cfg(not(target_os = "macos"))]
fn mac_dmg(_dmg: &Path, _staging: &Path, _exe: &str) -> Result<InstallOutcome, String> {
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
    fn app_bundle_comes_from_the_running_executable() {
        assert_eq!(app_bundle_for("/Applications/WordCraft.app/Contents/MacOS/wordcraft"), Some(PathBuf::from("/Applications/WordCraft.app")));
        assert_eq!(
            app_bundle_for("/Users/me/Applications/WordCraft.app/Contents/MacOS/wordcraft"),
            Some(PathBuf::from("/Users/me/Applications/WordCraft.app")),
            "bundle path is still discoverable; one-click install refuses non-/Applications targets"
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
        /// Whether the destination's folder is writable by this user (drives authorization).
        writable: bool,
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
        fn is_writable_dir(&self, path: &Path) -> bool {
            self.writable && self.dirs.borrow().contains(path)
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

    #[cfg(target_os = "macos")]
    #[test]
    fn dmg_install_refuses_bundles_outside_system_applications() {
        let dir = private_staging_dir().unwrap();
        let dmg = write_exclusive_file(&dir, "wordcraft.dmg", b"x").unwrap();
        let err = mac_dmg_with(&dmg, &dir, "/Users/me/Applications/WordCraft.app/Contents/MacOS/wordcraft", &RealDmgRunner, &RealAuthorizedBundle)
            .unwrap_err();
        assert!(err.contains("/Applications"), "{err}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn dmg_runner_detach_skipped_when_attach_fails() {
        use std::cell::Cell;
        struct FailAttach {
            detached: Cell<bool>,
        }
        impl DmgRunner for FailAttach {
            fn attach(&self, _: &str, _: &str) -> Result<(), String> {
                Err("attach failed".into())
            }
            fn detach(&self, _: &str) {
                self.detached.set(true);
            }
        }
        let runner = FailAttach { detached: Cell::new(false) };
        let dir = private_staging_dir().unwrap();
        let dmg = write_exclusive_file(&dir, "wordcraft.dmg", b"x").unwrap();
        let err = mac_dmg_with(&dmg, &dir, "/Applications/WordCraft.app/Contents/MacOS/wordcraft", &runner, &RealAuthorizedBundle).unwrap_err();
        assert!(err.contains("attach"), "{err}");
        assert!(!runner.detached.get(), "detach is only needed after a successful attach");
    }

    /// A fake bundle tree with `Contents/`, `Info.plist` and a runnable executable, so the
    /// authorized path validates the source the same way the direct path validates its copy.
    impl FakeOps {
        fn with_valid_bundle(self, root: &str) -> Self {
            let root = PathBuf::from(root);
            self.dirs.borrow_mut().insert(root.clone());
            self.dirs.borrow_mut().insert(root.join("Contents"));
            self.files.borrow_mut().insert(root.join("Contents/Info.plist"));
            self.dirs.borrow_mut().insert(root.join("Contents/MacOS"));
            let exe = root.join("Contents/MacOS/wordcraft");
            self.files.borrow_mut().insert(exe.clone());
            self.execs.borrow_mut().insert(exe);
            self
        }
    }

    /// An authorized handoff that records the call and reports a scripted result, so the fallback
    /// and its cancellation/failure paths are covered without touching `/Applications`.
    struct FakeAuth {
        result: Result<(), String>,
        calls: RefCell<Vec<(PathBuf, PathBuf)>>,
    }

    impl AuthorizedBundle for FakeAuth {
        fn replace(&self, staged: &Path, dest: &Path, exe_name: &str) -> Result<(), String> {
            let _ = exe_name;
            self.calls.borrow_mut().push((staged.to_path_buf(), dest.to_path_buf()));
            self.result.clone()
        }
    }

    fn install_ops(writable: bool) -> FakeOps {
        FakeOps { writable, ..FakeOps::default() }
            .with_dir("/apps")
            .with_valid_bundle("/apps/WordCraft.app")
            .with_valid_bundle("/stage/WordCraft.app")
    }

    #[test]
    fn writable_applications_uses_the_direct_swap_without_authorization() {
        let ops = install_ops(true);
        let auth = FakeAuth { result: Ok(()), calls: RefCell::new(Vec::new()) };
        install_bundle(&ops, &auth, &staged(), &dest(), "wordcraft").unwrap();
        assert!(ops.is_dir(&dest()), "the direct swap replaced the bundle");
        assert!(auth.calls.borrow().is_empty(), "no authorization when the folder is writable");
    }

    #[test]
    fn unwritable_applications_falls_back_to_authorization() {
        let ops = install_ops(false);
        let auth = FakeAuth { result: Ok(()), calls: RefCell::new(Vec::new()) };
        install_bundle(&ops, &auth, &staged(), &dest(), "wordcraft").unwrap();
        assert_eq!(auth.calls.borrow().len(), 1, "the authorized handoff replaced a read-only /Applications");
    }

    #[test]
    fn authorized_install_reports_cancellation() {
        let ops = install_ops(false);
        let auth = FakeAuth {
            result: Err("The update needs your permission to replace WordCraft in /Applications.".into()),
            calls: RefCell::new(Vec::new()),
        };
        let err = install_bundle(&ops, &auth, &staged(), &dest(), "wordcraft").unwrap_err();
        assert!(err.to_ascii_lowercase().contains("permission"), "{err}");
    }

    #[test]
    fn authorized_install_reports_permission_failure() {
        let ops = install_ops(false);
        let auth = FakeAuth { result: Err("operation not permitted".into()), calls: RefCell::new(Vec::new()) };
        let err = install_bundle(&ops, &auth, &staged(), &dest(), "wordcraft").unwrap_err();
        assert!(err.contains("not permitted"), "{err}");
        assert!(ops.is_dir(&dest()), "the installed bundle is untouched");
    }

    #[test]
    fn authorized_install_refuses_a_broken_download() {
        let ops =
            FakeOps { writable: false, ..FakeOps::default() }.with_dir("/apps").with_dir("/apps/WordCraft.app").with_dir("/stage/WordCraft.app");
        let auth = FakeAuth { result: Ok(()), calls: RefCell::new(Vec::new()) };
        let err = install_bundle(&ops, &auth, &staged(), &dest(), "wordcraft").unwrap_err();
        assert!(err.contains("incomplete") || err.contains("Info.plist"), "{err}");
        assert!(auth.calls.borrow().is_empty(), "an invalid bundle never reaches authorization");
    }

    #[test]
    fn authorized_script_quotes_paths_and_swaps_transactionally() {
        let script = authorized_replace_script(Path::new("/dmg dir/WordCraft.app"), Path::new("/Applications/WordCraft's.app"));
        assert!(script.contains("stage='/dmg dir/WordCraft.app'"), "a path with a space stays one shell word: {script}");
        assert!(script.contains(r"'/Applications/WordCraft'\''s.app'"), "a single quote is escaped, not ended: {script}");
        assert!(script.contains("ditto"), "the bundle is copied with ditto: {script}");
        assert!(
            script.contains("mv \"$dest\" \"$old\"") && script.contains("mv \"$old\" \"$dest\""),
            "the old bundle is restored on failure: {script}"
        );
    }

    #[test]
    fn apple_script_literal_escapes_quotes_and_backslashes() {
        let q = apple_quote("quotes \" and backslash \\ end");
        assert!(q.starts_with('"') && q.ends_with('"'), "wrapped as an AppleScript literal: {q}");
        assert!(q.contains("\\\""), "double quotes are escaped: {q}");
        assert!(q.contains("\\\\"), "backslashes are escaped: {q}");
    }

    /// Records the arguments and reports a scripted return code, so MSI behaviour is covered on
    /// any host (#15).
    struct FakeMsi {
        code: u32,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl MsiRunner for FakeMsi {
        fn run(&self, args: &[String], _timeout_secs: u64) -> Result<u32, String> {
            self.calls.borrow_mut().push(args.to_vec());
            Ok(self.code)
        }
    }

    fn run_msi(code: u32) -> (Result<InstallOutcome, String>, Vec<String>) {
        let fake = FakeMsi { code, calls: RefCell::new(Vec::new()) };
        let outcome = windows_msi_with(&fake, Path::new("C:\\tmp\\wordcraft.msi"));
        let args = fake.calls.into_inner().into_iter().next().unwrap_or_default();
        (outcome, args)
    }

    #[test]
    fn msi_args_suppress_restart_and_forced_shutdown() {
        let args = msi_args("pkg.msi");
        assert_eq!(args.first().map(String::as_str), Some("/i"));
        assert_eq!(args.get(1).map(String::as_str), Some("pkg.msi"));
        assert!(args.iter().any(|a| a == "/passive"), "{args:?}");
        assert!(args.iter().any(|a| a == "/norestart"), "an installer restart must be suppressed: {args:?}");
        assert!(
            args.iter().any(|a| a.eq_ignore_ascii_case("MSIRESTARTMANAGERCONTROL=DisableShutdown")),
            "the installer must not close WordCraft to replace files: {args:?}"
        );
    }

    #[test]
    fn msi_zero_is_an_install() {
        let (outcome, args) = run_msi(MSI_SUCCESS);
        assert_eq!(outcome.unwrap(), InstallOutcome::default());
        assert!(args.iter().any(|a| a == "/norestart"), "the runner receives the arguments: {args:?}");
        assert_eq!(msi_outcome(MSI_SUCCESS), MsiOutcome::Installed);
    }

    #[test]
    fn msi_3010_is_success_with_a_deferred_restart() {
        let (outcome, _) = run_msi(MSI_SUCCESS_REBOOT_REQUIRED);
        assert!(outcome.expect("3010 is success, not failure").reboot_required);
        assert_eq!(msi_outcome(MSI_SUCCESS_REBOOT_REQUIRED), MsiOutcome::InstalledRebootRequired);
        assert_eq!(msi_outcome(MSI_SUCCESS_REBOOT_INITIATED), MsiOutcome::InstalledRebootRequired);
    }

    #[test]
    fn msi_cancelled_is_an_error() {
        let (outcome, _) = run_msi(MSI_CANCELLED);
        let err = outcome.unwrap_err();
        assert!(err.to_ascii_lowercase().contains("cancel"), "{err}");
        assert_eq!(msi_outcome(MSI_CANCELLED), MsiOutcome::Cancelled);
    }

    #[test]
    fn msi_failure_is_an_error() {
        let (outcome, _) = run_msi(1603);
        let err = outcome.unwrap_err();
        assert!(err.contains("1603"), "{err}");
        assert_eq!(msi_outcome(1603), MsiOutcome::Failed(1603));
    }

    #[test]
    fn msi_runner_start_failure_is_propagated() {
        struct Failing;
        impl MsiRunner for Failing {
            fn run(&self, _args: &[String], _timeout_secs: u64) -> Result<u32, String> {
                Err("msiexec not found".into())
            }
        }
        assert!(windows_msi_with(&Failing, Path::new("x.msi")).is_err());
    }
}

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

use wordcraft_engine::update::{InstallFn, InstallHandoff, InstallKind};

pub fn handoff() -> InstallFn {
    std::sync::Arc::new(run)
}

fn run(job: InstallHandoff) -> Result<(), String> {
    let staging = private_staging_dir()?;
    let result = install_into(&job, &staging);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn install_into(job: &InstallHandoff, staging: &Path) -> Result<(), String> {
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

const INSTALLER_WAIT_SECS: u64 = 60 * 60;

#[cfg(target_os = "windows")]
fn windows_msi(path: &Path) -> Result<(), String> {
    let child = Command::new("msiexec").args(["/i", path.to_str().ok_or("bad path")?, "/passive"]).spawn().map_err(|e| e.to_string())?;
    let status = wait_child(child, INSTALLER_WAIT_SECS)?;
    if status.success() { Ok(()) } else { Err(format!("msiexec exited with {status}")) }
}

#[cfg(not(target_os = "windows"))]
fn windows_msi(_path: &Path) -> Result<(), String> {
    Err("MSI install is only on Windows".into())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_deb(path: &Path) -> Result<(), String> {
    let p = path.to_str().ok_or("bad path")?;
    let child = Command::new("pkexec").args(["dpkg", "-i", p]).spawn().map_err(|e| e.to_string())?;
    let status = wait_child(child, INSTALLER_WAIT_SECS)?;
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
}

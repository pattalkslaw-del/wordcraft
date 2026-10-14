//! One-click update handoff after a verified download (#15).

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
    let file_name = safe_name(&job.file_name);
    let path = write_exclusive_file(&staging, &file_name, &job.bytes)?;
    match job.kind {
        InstallKind::WindowsMsi => windows_msi(&path),
        InstallKind::WindowsPortableZip => windows_portable(&path, &job.exe_path, &staging),
        InstallKind::LinuxDeb => linux_deb(&path),
        InstallKind::MacDmg | InstallKind::MacApp => mac_dmg(&path, &staging),
        InstallKind::Flatpak | InstallKind::AppImage | InstallKind::Unknown => Err("This install type must use the release page.".into()),
    }
}

/// A private directory only this process can create (not a predictable shared path).
pub fn private_staging_dir() -> Result<PathBuf, String> {
    let mut id = [0u8; 16];
    getrandom::fill(&mut id).map_err(|e| e.to_string())?;
    let mut hex = String::with_capacity(32);
    for b in id {
        hex.push(hex_digit(b >> 4));
        hex.push(hex_digit(b & 0xf));
    }
    let path = std::env::temp_dir().join(format!("wordcraft-update-{hex}"));
    std::fs::create_dir(&path).map_err(|e| e.to_string())?;
    Ok(path)
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
    let dest = Path::new(exe);
    let parent = dest.parent().ok_or("could not find the running install folder")?;
    let pid = std::process::id();
    let script = staging.join("apply-portable.ps1");
    let ps = r#"param([int]$WaitPid, [string]$Src, [string]$Dest, [string]$Exe)
$ErrorActionPreference = 'Stop'
Wait-Process -Id $WaitPid -ErrorAction SilentlyContinue
robocopy $Src $Dest /E /NFL /NDL /NJH /NJS /nc /ns /np | Out-Null
if ($LASTEXITCODE -ge 8) { exit 1 }
Start-Process -FilePath $Exe
"#;
    std::fs::write(&script, ps).map_err(|e| e.to_string())?;
    let child = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            script.to_str().ok_or("bad path")?,
            "-WaitPid",
            pid.to_string(),
            "-Src",
            extract.to_str().ok_or("bad path")?,
            "-Dest",
            parent.to_str().ok_or("bad path")?,
            "-Exe",
            exe,
        ])
        .spawn()
        .map_err(|e| e.to_string())?;
    if child.id() == 0 {
        return Err("Could not start the portable update helper.".into());
    }
    Ok(())
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

#[cfg(target_os = "macos")]
fn mac_dmg(dmg: &Path, staging: &Path) -> Result<(), String> {
    let mount = staging.join("mnt");
    std::fs::create_dir(&mount).map_err(|e| e.to_string())?;
    let dmg_s = dmg.to_str().ok_or("bad path")?;
    let mount_s = mount.to_str().ok_or("bad path")?;
    let attach = Command::new("hdiutil").args(["attach", "-nobrowse", "-mountpoint", mount_s, dmg_s]).status().map_err(|e| e.to_string())?;
    if !attach.success() {
        return Err("Could not open the disk image.".into());
    }
    let app_src = mount.join("WordCraft.app");
    if !app_src.is_dir() {
        let _ = Command::new("hdiutil").args(["detach", mount_s]).status();
        return Err("The disk image does not contain WordCraft.app.".into());
    }
    let dest = Path::new("/Applications/WordCraft.app");
    if dest.exists() {
        let rm = Command::new("rm").args(["-rf", dest.to_str().ok_or("bad path")?]).status().map_err(|e| e.to_string())?;
        if !rm.success() {
            let _ = Command::new("hdiutil").args(["detach", mount_s]).status();
            return Err("Could not replace the existing WordCraft.app.".into());
        }
    }
    let cp = Command::new("cp").args(["-R", app_src.to_str().ok_or("bad path")?, "/Applications/"]).status().map_err(|e| e.to_string())?;
    let detach = Command::new("hdiutil").args(["detach", mount_s]).status().map_err(|e| e.to_string())?;
    if !cp.success() {
        return Err("Could not copy WordCraft.app to Applications.".into());
    }
    if !detach.success() {
        return Err("Could not unmount the disk image.".into());
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn mac_dmg(_dmg: &Path, _staging: &Path) -> Result<(), String> {
    Err("disk image install is only on macOS".into())
}

#[cfg(test)]
mod tests {
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
}

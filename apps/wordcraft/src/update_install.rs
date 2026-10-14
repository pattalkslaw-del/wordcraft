//! One-click update handoff after a verified download (#15).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use wordcraft_engine::update::{InstallFn, InstallHandoff, InstallKind};

pub fn handoff() -> InstallFn {
    std::sync::Arc::new(run)
}

fn run(job: InstallHandoff) -> Result<(), String> {
    let dir = temp_dir()?;
    let path = dir.join(safe_name(&job.file_name));
    {
        let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        f.write_all(&job.bytes).map_err(|e| e.to_string())?;
    }
    match job.kind {
        InstallKind::WindowsMsi => windows_msi(&path),
        InstallKind::WindowsPortableZip => windows_portable(&path, &job.exe_path),
        InstallKind::LinuxDeb => linux_deb(&path),
        InstallKind::MacDmg | InstallKind::MacApp => mac_dmg(&path),
        InstallKind::Flatpak | InstallKind::AppImage | InstallKind::Unknown => Err("this install type must use the release page".into()),
    }
}

fn temp_dir() -> Result<PathBuf, String> {
    let base = std::env::temp_dir().join("wordcraft-update");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    Ok(base)
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
fn windows_portable(zip: &Path, exe: &str) -> Result<(), String> {
    let extract = temp_dir()?.join("extract");
    let _ = std::fs::remove_dir_all(&extract);
    std::fs::create_dir_all(&extract).map_err(|e| e.to_string())?;
    let status = Command::new("tar")
        .args(["-xf", zip.to_str().ok_or("bad path")?, "-C", extract.to_str().ok_or("bad path")?])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("could not unpack the portable zip".into());
    }
    let dest = Path::new(exe);
    let parent = dest.parent().ok_or("could not find the running install folder")?;
    for entry in std::fs::read_dir(&extract).map_err(|e| e.to_string())? {
        let e = entry.map_err(|e| e.to_string())?;
        let name = e.file_name();
        let to = parent.join(name);
        if e.path().is_dir() {
            let _ = std::fs::remove_dir_all(&to);
            copy_dir(&e.path(), &to)?;
        } else {
            let _ = std::fs::remove_file(&to);
            std::fs::copy(e.path(), &to).map_err(|e| e.to_string())?;
        }
    }
    let _ = Command::new(exe).spawn();
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn windows_portable(_zip: &Path, _exe: &str) -> Result<(), String> {
    Err("portable zip update is only on Windows".into())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_deb(path: &Path) -> Result<(), String> {
    let p = path.to_str().ok_or("bad path")?;
    let status = Command::new("pkexec").args(["dpkg", "-i", p]).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err("dpkg install was cancelled or failed".into()) }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn linux_deb(_path: &Path) -> Result<(), String> {
    Err(".deb install is only on Linux".into())
}

#[cfg(target_os = "macos")]
fn mac_dmg(dmg: &Path) -> Result<(), String> {
    let mount = temp_dir()?.join("mnt");
    let _ = std::fs::remove_dir_all(&mount);
    std::fs::create_dir_all(&mount).map_err(|e| e.to_string())?;
    let script = format!(
        "hdiutil attach -nobrowse -mountpoint '{}' '{}' && cp -R '{}/WordCraft.app' /Applications/ && hdiutil detach '{}'",
        mount.display(),
        dmg.display(),
        mount.display(),
        mount.display()
    );
    let status = Command::new("osascript").args(["-e", &script]).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err("could not install from the disk image".into()) }
}

#[cfg(not(target_os = "macos"))]
fn mac_dmg(_dmg: &Path) -> Result<(), String> {
    Err("disk image install is only on macOS".into())
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

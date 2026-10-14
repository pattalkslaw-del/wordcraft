# Desktop updates

WordCraft checks [GitHub Releases](https://github.com/storytold/wordcraft/releases) for a newer version. The check is optional and privacy-preserving.

## What is sent

When you choose **Check for updates** (Help ribbon, File backstage, `file.checkForUpdates` — including through the MCP server — or `wordcraft-cli update --check`), WordCraft makes a single anonymous HTTPS `GET` to:

`https://api.github.com/repos/storytold/wordcraft/releases/latest`

No account, identifier, document content, or telemetry is sent. GitHub sees a normal API request from your IP, as any browser would.

## When checks run

- **Check for updates on startup** (File › Options › General) is **off** by default. On first launch the app asks once whether to turn it on.
- With the setting on, at most **one** check runs per 24 hours (failed checks count toward the interval too).
- Flatpak and AppImage builds use their own updaters; WordCraft still shows a notice when a newer version exists but does not download installers for those formats.

## One-click update

For `.msi`, portable `.zip`, `.deb`, and macOS `.dmg` installs, **Update** downloads the matching release asset, verifies it against `SHA256SUMS.txt` on the release, then starts the platform installer (`msiexec`, `dpkg` via `pkexec`, disk image copy, or portable replace-and-relaunch). If the document has unsaved changes, WordCraft asks you to save first, and the download does not start until it is saved. The macOS swap validates the downloaded bundle (`Contents/`, `Info.plist`, and the executable) before replacing anything, keeping the previous installation when validation fails.

The Windows portable update validates that the download carries `wordcraft.exe`, stages a validated copy on the installation's own volume, then renames the old installation aside and moves the new one into place. If the swap fails it restores the previous installation and relaunches it, so a failed update never leaves a half-replaced folder. Staged downloads, the helper and any backup are removed after use, including failed attempts.

A manual **Check for updates** answers every outcome: a newer release shows the notice bar, "up to date" and network errors appear in the status bar.

Unknown or development installs open the release page in your browser instead. On Linux, only `.deb` packages installed under system paths (verified with `dpkg-query`) use one-click install; RPM and unpackaged `/usr/bin` copies fall back to the release page. On Windows, a portable copy is recognised by its `-portable` folder wherever it lives (including under Program Files); a bare copy under Program Files is treated as an MSI install only when the installer's own `App Paths` registry entry names it, and is otherwise handed off to the release page rather than the wrong installer.

## Disabling checks

- Turn off **Check for updates on startup** in File › Options.
- Set `WORDCRAFT_NO_UPDATE_CHECK=1` (or any non-empty value other than `0`) for a single run, or
  compile with it set for packagers who ship updates another way. Automatic startup checks make no
  request at all; a manual **Check for updates** still works.

# Desktop updates

WordCraft checks [GitHub Releases](https://github.com/storytold/wordcraft/releases) for a newer version. The check is optional and privacy-preserving.

## What is sent

When you choose **Check for updates** (Help ribbon, File backstage, `file.checkForUpdates`, or `wordcraft-cli update --check`), WordCraft makes a single anonymous HTTPS `GET` to:

`https://api.github.com/repos/storytold/wordcraft/releases/latest`

No account, identifier, document content, or telemetry is sent. GitHub sees a normal API request from your IP, as any browser would.

## When checks run

- **Check for updates on startup** (File › Options › General) is **off** by default. On first launch the app asks once whether to turn it on.
- With the setting on, at most **one** check runs per 24 hours (failed checks count toward the interval too).
- Flatpak and AppImage builds use their own updaters; WordCraft still shows a notice when a newer version exists but does not download installers for those formats.

## One-click update

For `.msi`, portable `.zip`, `.deb`, and macOS `.dmg` installs, **Update** downloads the matching release asset, verifies it against `SHA256SUMS.txt` on the release, then starts the platform installer (`msiexec`, `dpkg` via `pkexec`, disk image copy, or portable replace-and-relaunch). If the document has unsaved changes, WordCraft asks you to save first.

Unknown or development installs open the release page in your browser instead.

## Disabling checks

- Turn off **Check for updates on startup** in File › Options.
- Set `WORDCRAFT_NO_UPDATE_CHECK=1` for a single run, or build with `WORDCRAFT_NO_UPDATE_CHECK=1` for packagers who ship updates another way.

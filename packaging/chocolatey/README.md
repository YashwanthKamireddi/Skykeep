# Chocolatey

`skykeep.nuspec` and `tools/` make the `skykeep` package for the Chocolatey Community
Repository. It downloads the release's per-user NSIS installer from GitHub Releases, checks
its SHA256 and runs it silently (`/S`); `chocolateyuninstall.ps1` runs Skykeep's own
uninstaller and waits for it to finish. No binaries are embedded, so no `VERIFICATION.txt` or
`LICENSE.txt` is needed in the package.

Proven by `.github/workflows/packages.yml` (Actions → Packages → Run workflow): `choco pack`,
`choco install skykeep --source .`, the installed version checked, `choco uninstall`. Its
artifact `windows-packages-<version>` holds the packed `.nupkg` for that version.

## Account

A free account at <https://community.chocolatey.org/account/Register>. The nuspec's
`<owners>` is the account's username, `yash_on_chocolatey`. The API key
is under <https://community.chocolatey.org/account>.

## Publishing a release

1. Run the Packages workflow with the version (for example `0.1.3`) and wait for green. It
   rewrites the version and installer SHA256 (from the release's `SHA256SUMS.txt`) in
   `skykeep.nuspec` and `tools/chocolateyinstall.ps1` and packs them.
2. Make the same change here and commit it: in `skykeep.nuspec` the `<version>` and the two
   `v0.1.2` in `<iconUrl>` and `<releaseNotes>`; in `tools/chocolateyinstall.ps1` the
   version in `url64bit` and `checksum64` (the `Skykeep_<ver>_x64-setup.exe` line of
   `SHA256SUMS.txt`).
3. On any Windows machine with Chocolatey, from this folder (or with the `.nupkg` from the
   workflow's artifact):

   ```powershell
   choco pack
   choco apikey add --source https://push.chocolatey.org/ --key <your API key>
   choco push skykeep.<version>.nupkg --source https://push.chocolatey.org/
   ```

4. The package goes through the automatic validator, verifier and virus scan, then a human
   moderator for the first versions; the package page shows each result. Answer the
   moderator's comments there. Once a few versions are approved, later ones are usually
   approved automatically.

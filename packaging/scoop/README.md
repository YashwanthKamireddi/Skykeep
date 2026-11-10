# Scoop

`skykeep.json` is the Scoop manifest. Scoop Extras asks for an established project (100+
stars), so until then it is published from a bucket of your own. It unpacks the release's NSIS
installer with 7-Zip instead of running it (the way Extras ships Tauri apps), so everything
stays in Scoop's folder, with Start menu shortcuts for Skykeep and its six apps. (The
installer's `.ics`/`.vcf` file associations are not registered this way.) No `persist`: Skykeep keeps its library and settings in your profile
(`%APPDATA%`, `%LOCALAPPDATA%`), never in its own folder, so they stay across updates.
`checkver` reads the latest GitHub release and `autoupdate` takes the installer's hash from
the release's `SHA256SUMS.txt`.

Releases after 0.1.2 see Scoop's `scoop-install.json` beside `skykeep.exe` and leave updates to
Scoop (`src-tauri/src/updates.rs`); 0.1.2 itself still offers its own update.

Proven by `.github/workflows/packages.yml` (Actions → Packages → Run workflow): `scoop install`
from this manifest, the folder and the seven shortcuts checked, Skykeep started, `checkver`
and `autoupdate` reproducing this manifest for the release, `scoop uninstall`.

## Account

Only GitHub. A public repository named `scoop-bucket` (any name works) on
`github.com/YashwanthKamireddi`.

## Publishing

Once, make the bucket:

```sh
gh repo create YashwanthKamireddi/scoop-bucket --public \
  --description "Scoop bucket for Skykeep"
git clone https://github.com/YashwanthKamireddi/scoop-bucket
mkdir scoop-bucket/bucket
cp packaging/scoop/skykeep.json scoop-bucket/bucket/
cd scoop-bucket && git add . && git commit -m "skykeep 0.1.2" && git push
```

People then install it with:

```powershell
scoop bucket add skykeep https://github.com/YashwanthKamireddi/scoop-bucket
scoop install skykeep/skykeep
```

At each published release, `.github/workflows/stores.yml` moves `bucket/skykeep.json` to it
(version, URL and hash) and pushes it, with the `SCOOP_BUCKET_TOKEN` secret (see
[`../README.md`](../README.md#from-a-release-to-every-store)). By hand, on Windows with Scoop,
update the manifest from the release and push it (and copy it back here):

```powershell
& "$(scoop prefix scoop)\bin\checkver.ps1" -App skykeep -Dir .\bucket -Update
git commit -am "skykeep <version>"; git push
```

Later, with 100+ stars, the same manifest can be proposed to Scoop Extras as a pull request to
`ScoopInstaller/Extras` (`bucket/skykeep.json`); its CI runs the same checks.

# Release & Code-Signing Playbook

This document captures the exact commands used to produce signed installers
for AeroTab. It is meant for the project release engineer; nothing here
runs automatically during day-to-day development.

## 0. Versioning

Bump the version in three places, in this order:

1. `Cargo.toml` (workspace `[workspace.package].version`)
2. `src-tauri/tauri.conf.json` (`"version"`)
3. `apps/ui/package.json` (`"version"`)

Tag the release commit `vX.Y.Z`. The updater feed (`latest.json`) MUST be
regenerated for every tagged release — see §4.

## 1. Updater signing key

AeroTab ships with a Minisign keypair embedded in `tauri.conf.json`
(`plugins.updater.pubkey`). The matching **private** key is required to sign
bundles so existing installs trust the update.

```bash
# One-time, kept offline (e.g. hardware token / 1Password):
cargo tauri signer generate -w ~/.secrets/aerotab-updater.key
# Set a password when prompted; you'll need it for every release.
```

For every release:

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.secrets/aerotab-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD='...'   # or use `gpg --decrypt`
```

The dev keypair under `.tauri/dev-updater.key` is **for local self-tests
only** — never publish artifacts signed with it.

## 2. Linux (.deb / AppImage)

```bash
cd src-tauri
cargo tauri build --bundles deb appimage
```

Optional GPG sign of the `.deb` (Debian repo style):

```bash
dpkg-sig --sign builder \
  -k <KEYID> \
  target/release/bundle/deb/Tabby\ v2_*_amd64.deb
```

If you publish a Debian repo, regenerate `Release` + sign it with
`apt-ftparchive release ... | gpg --clearsign > InRelease`.

## 3. Windows (NSIS) — EV code-signing

The Tauri bundler does not sign on non-Windows hosts (it warns and skips).
For an authentic Windows release you have two paths:

### 3a. Build + sign on a Windows host

```powershell
$Env:TAURI_SIGNING_PRIVATE_KEY = Get-Content .\.secrets\aerotab-updater.key -Raw
$Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '...'
cargo tauri build --bundles nsis msi

# Sign with the EV certificate stored on a hardware token (e.g. SafeNet):
$installer = 'target\release\bundle\nsis\AeroTab_*_x64-setup.exe'
signtool sign /tr http://timestamp.digicert.com /td sha256 /fd sha256 `
  /n "<EV Subject CN>" $installer
signtool verify /pa /v $installer
```

### 3b. Cross-compile from Linux, sign separately

```bash
cd src-tauri
cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc \
  --bundles nsis -- --features desktop
```

Copy the produced `.exe` to a Windows signing host (or a CI runner with
access to the HSM-backed EV cert) and run `signtool` as in §3a.

The bundler also writes a `*.exe.sig` next to the installer — that is the
**Minisign** signature consumed by the updater, **not** Authenticode. Both
must be present in the final release artifact set.

## 4. Updater feed (`latest.json`)

Produce one feed per platform that the updater plugin will fetch from
`plugins.updater.endpoints` (configured in `tauri.conf.json`). Minimal
format:

```json
{
  "version": "0.2.0",
  "notes": "See CHANGELOG.md",
  "pub_date": "2025-05-22T02:30:00Z",
  "platforms": {
    "linux-x86_64": {
      "signature": "<contents of AeroTab_0.2.0_amd64.deb.sig>",
      "url": "https://releases.aerotab.example/v0.2.0/Tabby%20v2_0.2.0_amd64.deb"
    },
    "windows-x86_64": {
      "signature": "<contents of AeroTab_0.2.0_x64-setup.exe.sig>",
      "url": "https://releases.aerotab.example/v0.2.0/Tabby%20v2_0.2.0_x64-setup.exe"
    }
  }
}
```

For a local self-test:

```bash
# Serve target/release/bundle as if it were a release host.
python3 -m http.server -d target 8080
# Then edit tauri.conf.json's endpoints to:
#   http://localhost:8080/release/latest.json
```

Open the app → **Settings → Updates → Check for updates** to validate the
end-to-end flow.

## 5. macOS (out of scope for v2.0)

The first releases ship Linux + Windows only. When macOS lands, follow
Apple's notarization path:

```bash
cargo tauri build --bundles app dmg
codesign --deep --force --options runtime --timestamp \
  --sign "Developer ID Application: <Name> (<TEAMID>)" \
  "target/release/bundle/macos/AeroTab.app"
xcrun notarytool submit \
  "target/release/bundle/dmg/AeroTab_0.2.0_x64.dmg" \
  --apple-id <id> --team-id <TEAMID> --keychain-profile tabby-notary --wait
xcrun stapler staple "target/release/bundle/dmg/AeroTab_0.2.0_x64.dmg"
```

## 6. Release checklist

- [ ] Bump version in the three files above.
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --features desktop -- -D warnings`
- [ ] `cargo test --lib --features desktop`
- [ ] `cd apps/ui && npm run build`
- [ ] Build Linux bundles (§2) with signing env vars set.
- [ ] Build Windows bundles (§3) on Windows host or cross-compile + sign.
- [ ] Verify `.sig` exists for every bundle in `target/.../bundle/`.
- [ ] Authenticode-sign Windows installer (`signtool verify /pa`).
- [ ] Upload artifacts to the release host; generate `latest.json` (§4).
- [ ] Smoke-test in-app updater: install previous release, hit
      **Settings → Updates → Check**, confirm download + restart.
- [ ] Tag `vX.Y.Z` in git, push.

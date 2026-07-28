# Kotone Windows installer

Kotone keeps Tauri's NSIS installation behavior and uses only NSIS Modern UI 2
features plus repository-owned artwork. Users can still choose the installation
directory.

There are no proprietary installer dependencies. A clean Windows CI runner can
build the installer through the normal Tauri CLI flow.

Pull-request CI uses `tauri.ci.conf.json` to compile a debug NSIS installer
without updater artifacts. Tagged releases keep the normal updater signing
configuration and receive the private key from repository secrets.

## Assets

To rebuild the checked-in Modern UI sidebar and header artwork from the brand
assets:

```powershell
.\generate-assets.ps1
```

`src/assets/brand/kotone-cutout.png` is the transparent copy of the latest
approved RepoChan cutout and is shared with the app UI. Keep that source and the
generated installer bitmaps in the same change when the character delivery is
updated.

## Tauri upgrades

`kotone-installer.nsi` is based on Tauri CLI 2.11.4's upstream NSIS template.
When Tauri is upgraded, diff the new upstream template and carry the Kotone MUI
theme and copy changes forward before shipping. Application installation,
registry, shortcuts, WebView2, reinstall, and uninstall behavior remain owned by
Tauri's template.

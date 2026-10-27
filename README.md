# ParameterSaveStates (Rust)

Rust rewrite of ParameterSaveStates for **Windows and Linux**. It has a native
[egui](https://github.com/emilk/egui) UI and no browser or Unity dependency.
The Unity project in the repository root stays until this version reaches feature parity.

## Crates

| Crate | Purpose |
|---|---|
| `psstates-core` | Data model, JSON storage, matching rules, legacy migration, backup zip |
| `psstates-osc` | OSC (UDP), OSCQuery client and server, and mDNS discovery |
| `psstates-service` | Connects VRChat to the store: avatar changes, auto-sync, fixed parameters, save and apply |
| `psstates-ui` | egui UI that renders into any `egui::Context` (desktop window and VR overlay) |
| `psstates-vr` | Optional SteamVR dashboard overlay: dynamically loaded OpenVR bindings and a CPU renderer |
| `psstates-app` | `ParameterSaveStates` binary: window, tray icon, single instance, CLI |
| `xtask` | `cargo xtask package` builds the release archives |

## Build and run

```sh
cd rust
cargo run --release
```

Linux needs the usual winit/egui build dependencies, for example on Debian or Ubuntu:
`libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev`.

- **SteamVR overlay:** enabled by default through the cargo feature `openvr`. Build with
  `--no-default-features` to remove it.
- **OpenVR library:** `openvr_api.dll` (Windows) or `libopenvr_api.so` (Linux) is loaded at runtime from
  next to the executable, or from `PSSTATES_OPENVR_LIB`. Without it the app runs normally
  without VR. For development, copy it next to the built executable, for example
  `Assets/OVRLay/Runtime/OpenVR/Plugins/x86_64/openvr_api.dll` into `target/debug/`.

### Packaging

```sh
cargo xtask package --openvr-lib path/to/openvr_api.dll   # or libopenvr_api.so
```

This creates `dist/<platform>/` and `dist/ParameterSaveStates-windows.zip` or
`dist/ParameterSaveStates-linux-x86_64.tar.gz`. Each contains the binary, `app.vrmanifest`,
the icon and the OpenVR library. The Linux archive also contains `install.sh` and a `.desktop` file.

The release workflow (`.github/workflows/rust-release.yml`, triggered by `rust-v*` tags) builds both
platforms with OpenVR SDK v2.0.10. On Windows it also builds an Inno Setup installer
(`packaging/windows/installer.iss`).

### Live SteamVR tests

Some tests need a running SteamVR and are ignored by default:

```sh
PSSTATES_OPENVR_LIB=path/to/openvr_api.dll cargo test -p psstates-vr --release live_ -- --ignored --nocapture --test-threads=1
```

They time both upload paths and check that a rejected texture falls back to raw pixels.

### CLI

```text
ParameterSaveStates                 start the app
ParameterSaveStates --minimized     start in the tray (SteamVR uses this)
ParameterSaveStates migrate [DIR]   import data of the old Unity version
ParameterSaveStates --data-dir      print the data directory
ParameterSaveStates --version
```

## Desktop and tray behaviour

- **Startup:** if SteamVR is running when the app starts, or when it is started with
  `--minimized`, the app starts in the tray. Otherwise it opens the window, as the old version did.
- **Closing the window:** the app keeps running in the tray, where *Open* and *Exit* are available.
  Without a tray (for example a Linux desktop without StatusNotifierItem support) and without
  SteamVR, closing the window exits the app.
- **Starting it again:** a second launch brings the running window to the front. The app uses
  `127.0.0.1:17663` as its instance lock.
- **SteamVR shutdown:** if the app was started in the background for SteamVR, it exits when SteamVR
  shuts down.
- **Log file:** `ParameterSaveStates.log` in the data directory.

## SteamVR dashboard overlay

- **What it shows:** the full UI inside the SteamVR dashboard (key `parametersavestates_menu`,
  2.6 m wide, 1920×1200). It is rendered on the CPU, so it does not depend on a graphics API for
  drawing. It redraws only while visible and something changes, at about 30 fps at most, and a frame
  identical to the previous one is never uploaded.
- **Upload:** on Windows the frame goes to SteamVR as a Direct3D 11 texture on SteamVR's GPU
  (`SetOverlayTexture`), which avoids the flicker that `SetOverlayRaw` causes. If that is not possible
  it falls back to `SetOverlayRaw`, which Linux always uses.
- **Laser pointer:** hover animations and tooltips are off in VR, because a pointer is never perfectly
  still and every changed frame is an upload.
- **Input:** laser pointer clicks and scrolling work. Focusing a text field opens the SteamVR keyboard.
- **Registration:** on every connect the app registers `app.vrmanifest` from next to the executable,
  so it appears under SteamVR's startup overlay apps.
- **Start with SteamVR:** this can be switched on in *Settings → General → SteamVR*.
- **Not available in VR:** backup, restore and legacy import, because they need file dialogs.

## Data

- **Windows:** `%APPDATA%\I5UCC\ParameterSaveStates\data`
- **Linux:** `~/.local/share/parametersavestates`
- **Override:** set the `PSSTATES_DATA_DIR` environment variable.

Files:

```text
settings.json          global filters, auto-sync, fixed parameters and theme
avatars/<avatar>.json  name, ordered profiles, per-avatar filters, auto-sync, fixed parameters
```

### Migrating from the Unity version

- **On first start:** if old data is found in
  `%USERPROFILE%\AppData\LocalLow\I5UCC\ParameterSaveStates\Profiles`, the app offers to import it.
- **Manually:** use *Settings → General → Import from the old (Unity) version*, or run
  `ParameterSaveStates migrate <Profiles folder>`.
- **From an old backup:** a zip exported by the old web UI can be imported directly with
  *Import backup*.
- **Name clashes:** existing profiles are never overwritten. An imported profile whose
  name is already taken gets a `(legacy)` suffix.

## How it talks to VRChat

1. **Discovery:** the app advertises `_osc._udp` and `_oscjson._tcp` via mDNS as
   `VRCParameterSaveStates`. It serves a small OSCQuery tree that asks for `/avatar/change`,
   and browses for `VRChat-Client-*` OSCQuery services.
2. **Connection:** when VRChat is found, the app reads `HOST_INFO` and sends OSC to the
   `OSC_IP:OSC_PORT` given there. If that is unavailable it falls back to `127.0.0.1:9000`.
3. **Saving:** the current values are read from VRChat's `/avatar/parameters` OSCQuery node.
4. **Avatar names:** taken from VRChat's `/avatar/name` OSCQuery node and cached per avatar.
5. **Avatar changes:** on each change, auto-sync values from the previous avatar are sent
   first, then the fixed parameters.

## Status and roadmap

- [x] Core storage and rules, legacy migration, backup export and import
- [x] OSC, OSCQuery and mDNS connection handling
- [x] Desktop UI: profiles, filters, auto-sync, fixed parameters, backup, legacy import
- [x] System tray, start minimised, single instance, log file
- [x] OpenVR dashboard overlay (cargo feature `openvr`, reusing `psstates-ui`)
- [x] Release packaging (Windows zip and installer, Linux tar.gz) and SteamVR manifest with a Linux binary
- [ ] Optional HTTP API (cargo feature `http-api`)
- [ ] Linux AppImage / Flatpak

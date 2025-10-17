# Loopback DriverKit Workspace

This repository bundles everything you need to build, install, and exercise a
virtual Core Audio device that routes multiple audio sources into a stereo
loopback that Zoom (or any Core Audio client) can see. The workspace contains:

- `device_kit/` – the Rust mixer core, CLI tools, and Node/Electron bindings.
- `SwiftUI/` – the SwiftUI control-panel sources that let you monitor levels and
  tweak gain/mute state.
- `AudioServerPlugInTests/` – Xcode XCTest stubs that talk to the exported C FFI.

The instructions below walk a non-specialist through prerequisites, driver
installation, and basic testing.

---
## 1. Prerequisites

| Requirement | Notes |
|-------------|-------|
| **Apple Developer Program account** | Needed so Xcode can sign the DriverKit extension. |
| **macOS 14+ (Sonoma/Sequoia)** | DriverKit audio requires recent macOS releases. |
| **Xcode 15+** | Includes the DriverKit SDK and `xcodebuild`. Install from the Mac App Store. |
| **Command Line Tools** | `xcode-select --install` if they are missing. |
| **Rust toolchain** | `curl https://sh.rustup.rs -sSf \
  | sh` (or use Homebrew `brew install rust`). |
| **Homebrew** (optional) | Only needed if the setup script installs `xcodegen` for you. |
| **Node.js** (optional) | Required only if you want to exercise the Electron/Node bridge. |

> ⚠️ You must grant the built driver in System Settings → Privacy & Security
during the install; macOS will prompt you automatically.

---
## 2. Clone and bootstrap

```bash
# Use SSH or HTTPS as you prefer
git clone <repo-url>
cd <repo>/driver_kit
```

All code now lives inside `driver_kit/` – there are no stray sources outside
this directory.

---
## 3. Build the Rust mixer and Xcode project

Run the helper script (it compiles the Rust dylib, gathers the SwiftUI files,
creates entitlements, and generates an Xcode project via `xcodegen`):

```bash
cd driver_kit/device_kit
scripts/setup_driver.sh
```

If `xcodegen` is missing, the script installs it with Homebrew. On success you
should see:

```
==> Ready.
Open the generated Xcode project at:
  driver_kit/device_kit/build/LoopbackDriver/LoopbackDriver.xcodeproj
```

---
## 4. Sign, build, and run the DriverKit extension

1. Open the generated project:
   ```bash
   open build/LoopbackDriver/LoopbackDriver.xcodeproj
   ```
2. In Xcode select the **LoopbackDriverExtension** target and set your Team ID.
   Do the same for **LoopbackDriverApp** (the host app).
3. Choose the scheme **LoopbackDriverExtension**, set the run destination to
   “Any Mac (DriverKit)”, and build (`⌘B`).
4. Run the **LoopbackDriverApp** scheme once – the host app activates the
   extension. macOS should prompt you to approve the driver in
   **System Settings → Privacy & Security**. Approve and reboot if asked.

After approval, the driver stays registered system-wide. You don’t need to keep
the host app running.

---
## 5. Verify from the command line

With the driver active, the Rust CLI can poll its status:

```bash
# Shows sample rate, latency, drift, and per-source meters
cargo run --bin loopbackctl -- --status

# Launch the interactive ratatui console (Up/Down/g/m/q)
cargo run --bin loopbackctl
```

The CLI lives in `device_kit/src/bin/loopbackctl.rs`. It uses the same FFI
API that the driver and control panel call.

---
## 6. SwiftUI control panel (optional)

The `SwiftUI/` directory contains a ready-to-drop control panel. You can:

1. Create a new macOS SwiftUI app in Xcode.
2. Add all files from `driver_kit/SwiftUI/` to the project.
3. Add `driver_kit/device_kit/LoopbackBridge.h` as the bridging header and link
   against the release `libdevice_kit.dylib`
   (`driver_kit/device_kit/target/release/libdevice_kit.dylib`).
4. Run the app – it polls level meters every 50 ms and lets you adjust
   gain/mute per source.

---
## 7. Electron / Node integration (optional)

A ready-made Node addon lives in `device_kit/napi-rs`:

```bash
cd driver_kit/device_kit/napi-rs
npm install
npm run build    # produces index.node
node examples/node_feed.js   # streams a sine wave into the mixer
```

From your Electron app, load the addon (`require('./index.node')`) and call
`registerSource`, `pushAudioFrame`, `setSourceGain`, etc. Ensure the DriverKit
extension is already installed/approved.

---
## 8. Logs & diagnostics

- The render callback emits tracing events; the CoreAudio plug-in forwards them
to `os_log`. Run `log stream --predicate 'subsystem == "com.devicekit.loopback.device"'`
to watch them in real time.
- `loopbackctl --status` also dumps the latest RMS meters.
- A Rust sine-wave self-test (`cargo test` inside `device_kit/`) verifies the
  synchronous mix path when crates.io is available.

---
## 9. Uninstall / cleanup

To unload the driver during development:

```bash
# From within the running system extension helper (host app or CLI)
loopbackctl --status   # ensure driver is active
# Stop via the host app or CLI shortcut, then from Xcode use Product ▸ Stop.

# To remove completely use systemextensionsctl
sudo systemextensionsctl uninstall com.example.loopbackdriver.app com.example.loopbackdriver.extension
```

Replace the bundle identifiers with the ones you configure in Xcode.

---
## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Driver doesn’t appear in Audio MIDI Setup | Make sure the System Settings approval was granted and you rebooted after enabling. |
| `loopbackctl` reports “no active mixer” | The DriverKit extension isn’t running; start the host app or re-run the activation helper. |
| Build fails with missing crates | Run `cargo fetch` (or `cargo build`) once with internet access to cache dependencies. |
| Electron cannot load the addon | Ensure you ship the correct `.node` binary for macOS and that the driver is already approved. |

---
## Need to rebuild?

Re-run `scripts/setup_driver.sh` any time you change the Rust or SwiftUI code;
it copies the latest artifacts into `build/LoopbackDriver` and regenerates the
Xcode project.

That’s it—once the driver is approved, it loads automatically at boot and is
visible to Zoom as “Loopback Device”. Use the CLI, SwiftUI panel, or the Node
addon to monitor and route audio however you like.

# Loopback Audio Driver DriverKit Integration

This package ships the CoreAudio plug‑in sources (`LoopbackDevice.swift`, `LoopbackRender.cpp`) and the Rust mixer (`device_kit` crate). To make the virtual loopback device visible to macOS/Zoom you need to build and sign a DriverKit extension that links against the Rust `cdylib`, then install the signed bundle.

The steps below assume Xcode 15+, macOS 14+, and a registered Apple Developer Team ID.

---

## 1. Build the Rust Dynamic Library

```bash
cargo build --release
```

This produces `target/release/libdevice_kit.dylib`. We’ll embed that in the DriverKit extension bundle so the C++ render stub can call into Rust.

---

## 2. Create an Xcode DriverKit Project

1. Open Xcode → *File ▸ New ▸ Project…* → choose **DriverKit Extension**.
2. Name it `LoopbackDriver`, set the Organization Identifier to match your Team ID domain (e.g. `com.example`), language **Swift**, and check **Include Audio Driver entitlements**.
3. Xcode creates:
   - `LoopbackDriverExtension` target (DriverKit bundle)
   - `LoopbackDriverApp` host app (for driver deployment)

---

## 3. Add the Project Sources

Copy/ add the files from this repository into the Xcode project:

| Repo Path | Xcode Target | Notes |
|-----------|--------------|-------|
| `AudioServerPlugIn/LoopbackDevice.swift` | DriverKit extension | Add to Sources group |
| `AudioServerPlugIn/LoopbackRender.cpp` | DriverKit extension | Add to Sources; make sure the file is marked as Objective‑C++ (`.mm`) or set the compile file type to C++ |
| `LoopbackBridge.swift` | DriverKit extension | Add to Sources |
| `LoopbackBridge.h` | DriverKit extension | Set as the bridging header (Target ▸ Build Settings ▸ Swift Compiler – General ▸ Objective‑C Bridging Header) |

> **Tip:** DriverKit requires Objective‑C++ when mixing C++ and Obj‑C. Rename `LoopbackRender.cpp` to `LoopbackRender.mm` inside Xcode.

---

## 4. Link the Rust Library

1. Drag `target/release/libdevice_kit.dylib` into the DriverKit target (choose *Copy items if needed* when prompted).
2. In the driver target build settings:
   - **Other Linker Flags**: add `-ldevice_kit`.
   - **Library Search Paths**: add `$(PROJECT_DIR)/../path/to/target/release` (adjust relative path).
3. Ensure the bundle includes the dylib: Add a new *Copy Files Build Phase* targeting “Frameworks” and include `libdevice_kit.dylib`.

Every time the Rust code changes, rebuild (`cargo build --release`) and the dylib in Xcode will update.

---

## 5. Configure Entitlements & Info.plist

DriverKit needs explicit entitlements:

1. Update `LoopbackDriverExtension.entitlements`:
   ```xml
   <dict>
     <key>com.apple.developer.driverkit</key>
     <true/>
     <key>com.apple.developer.driverkit.transport.audio</key>
     <true/>
     <key>com.apple.developer.driverkit.family.audio</key>
     <true/>
   </dict>
   ```

2. In the extension’s `Info.plist`, add the Audio Server plug‑in class:
   ```xml
   <key>IOKitPersonalities</key>
   <dict>
     <key>LoopbackAudioDevice</key>
     <dict>
       <key>CFBundleIdentifier</key>
       <string>$(PRODUCT_BUNDLE_IDENTIFIER)</string>
       <key>IOClass</key>
       <string>LoopbackDevice</string>
       <key>IOMatchCategory</key>
       <string>IOUserAudio</string>
       <key>IOProviderClass</key>
       <string>IOAudioDevice</string>
     </dict>
   </dict>
   ```

The Swift `LoopbackDevice` implements the `IOUserAudioDriver` subclass specified above.

---

## 6. Signing & Provisioning

1. In both the extension and host app targets, set **Team** to your Apple Developer team.
2. Ensure the bundle identifier matches the provisioning profile (e.g. `com.example.LoopbackDriver`).
3. DriverKit extensions require a special “DriverKit” entitlement profile. Xcode generates this automatically when the target type is DriverKit and a Team is assigned.

---

## 7. Build & Install

1. Select the **LoopbackDriverApp** scheme and a “My Mac (Designed for iPad)” or “My Mac” destination.
2. Build and run. The host app installs the DriverKit extension.
3. Approve the driver in **System Settings ▸ Privacy & Security** if prompted, then reboot when macOS asks.

After restart, open **Audio MIDI Setup** – you should see **Loopback Output** and **Loopback Input** devices.

---

## 8. Zoom Validation

1. Launch Zoom.
2. Go to **Settings ▸ Audio** and choose “Loopback Input” as the microphone.
3. Play audio into the Node/Swift sources; the Rust mixer feeds those into the virtual input, and Zoom receives the mixed stream.

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Driver fails to load | Check Console.app for `LoopbackDevice` logs; ensure the dylib is codesigned along with the bundle |
| Zoom can’t see the device | Confirm the driver bundle is approved in Privacy & Security and that `IOAudioFamily` shows the device via `systemextensionsctl list` |
| Build errors about missing symbols | Verify `libdevice_kit.dylib` is included in the DriverKit extension and the bridging header path is correct |

For deeper debugging, enable DriverKit logging:

```bash
log stream --predicate 'subsystem == "com.devicekit.loopback.device"'
```

This surfaces the `Logger` output from `LoopbackDevice.swift`.

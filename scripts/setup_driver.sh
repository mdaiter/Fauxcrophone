#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"
OUTPUT_DIR="$PROJECT_ROOT/build/LoopbackDriver"
RUST_TARGET="$PROJECT_ROOT/target/release/libdevice_kit.dylib"
SWIFT_SOURCES_DIR="$PROJECT_ROOT/SwiftUI"
BRIDGE_HEADER="$PROJECT_ROOT/LoopbackBridge.h"

command_exists() {
  command -v "$1" >/dev/null 2>&1
}

echo "==> Checking prerequisites"
for cmd in cargo xcodebuild; do
  if ! command_exists "$cmd"; then
    echo "Error: '$cmd' is required but not installed." >&2
    exit 1
  fi
done

if ! command_exists xcodegen; then
  echo "Warning: xcodegen not found."
  echo "Installing xcodegen via Homebrew..."
  if command_exists brew; then
    brew install xcodegen
  else
    echo "Error: Homebrew is required to install xcodegen automatically." >&2
    exit 1
  fi
fi

mkdir -p "$OUTPUT_DIR"

cat <<'YAML' > "$OUTPUT_DIR/project.yml"
name: LoopbackDriver
options:
  deploymentTarget: 14.0
  xcodeVersion: 15.0
  createIntermediateDirectories: true
targets:
  LoopbackDriverApp:
    type: application
    platform: macOS
    sources:
      - path: HostApp
    settings:
      PRODUCT_BUNDLE_IDENTIFIER: com.example.loopbackdriver.app
      INFOPLIST_FILE: HostApp/Info.plist
      CODE_SIGN_STYLE: Automatic
    dependencies:
      - target: LoopbackDriverExtension

  LoopbackDriverExtension:
    type: driver-extension
    platform: macOS
    productName: LoopbackDriverExtension
    sources:
      - path: DriverExtension
    settings:
      PRODUCT_BUNDLE_IDENTIFIER: com.example.loopbackdriver.extension
      INFOPLIST_FILE: DriverExtension/Info.plist
      SWIFT_OBJC_BRIDGING_HEADER: DriverExtension/LoopbackBridge.h
      OTHER_LDFLAGS: "-ldevice_kit"
      LD_RUNPATH_SEARCH_PATHS: "$(PROJECT_DIR)/DriverExtension/Frameworks"
    buildPhases:
      - name: CopyRustDylib
        type: shellScript
        shellScript: |
          set -e
          mkdir -p "$TARGET_BUILD_DIR/$CONTENTS_FOLDER_PATH/Frameworks"
          cp "$PROJECT_DIR/DriverExtension/Frameworks/libdevice_kit.dylib" "$TARGET_BUILD_DIR/$CONTENTS_FOLDER_PATH/Frameworks/libdevice_kit.dylib"
        executionPosition: afterCompile
YAML

mkdir -p "$OUTPUT_DIR/DriverExtension/Frameworks"
mkdir -p "$OUTPUT_DIR/DriverExtension/Sources"
mkdir -p "$OUTPUT_DIR/DriverExtension"
mkdir -p "$OUTPUT_DIR/HostApp"

cp "$BRIDGE_HEADER" "$OUTPUT_DIR/DriverExtension/LoopbackBridge.h"
cp "$SWIFT_SOURCES_DIR/LoopbackBridge.swift" "$OUTPUT_DIR/DriverExtension/Sources/LoopbackBridge.swift"
cp "$SWIFT_SOURCES_DIR/DeviceView.swift" "$OUTPUT_DIR/DriverExtension/Sources/DeviceView.swift"
cp "$SWIFT_SOURCES_DIR/LoopbackViewModel.swift" "$OUTPUT_DIR/DriverExtension/Sources/LoopbackViewModel.swift"
cp "$SWIFT_SOURCES_DIR/MeterView.swift" "$OUTPUT_DIR/DriverExtension/Sources/MeterView.swift"
cp "$SWIFT_SOURCES_DIR/LoopbackControlApp.swift" "$OUTPUT_DIR/HostApp/LoopbackControlApp.swift"

cat <<'PLIST' > "$OUTPUT_DIR/HostApp/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>LoopbackDriverApp</string>
  <key>CFBundleIdentifier</key>
  <string>com.example.loopbackdriver.app</string>
  <key>CFBundleExecutable</key>
  <string>LoopbackDriverApp</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>NSPrincipalClass</key>
  <string>NSApplication</string>
</dict>
</plist>
PLIST

cat <<'PLIST' > "$OUTPUT_DIR/DriverExtension/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>com.example.loopbackdriver.extension</string>
  <key>IOKitPersonalities</key>
  <dict>
    <key>LoopbackAudioDevice</key>
    <dict>
      <key>CFBundleIdentifier</key>
      <string>com.example.loopbackdriver.extension</string>
      <key>IOClass</key>
      <string>LoopbackDevice</string>
      <key>IOMatchCategory</key>
      <string>IOUserAudio</string>
      <key>IOProviderClass</key>
      <string>IOAudioDevice</string>
    </dict>
  </dict>
</dict>
</plist>
PLIST

cat <<'PLIST' > "$OUTPUT_DIR/DriverExtension/LoopbackDriverExtension.entitlements"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.developer.driverkit</key>
  <true/>
  <key>com.apple.developer.driverkit.transport.audio</key>
  <true/>
  <key>com.apple.developer.driverkit.family.audio</key>
  <true/>
</dict>
</plist>
PLIST

cat <<'SWIFT' > "$OUTPUT_DIR/DriverExtension/Sources/LoopbackDevice.swift"
import AudioServerPlugIn
import AVFoundation
import os.log

// Placeholder device implementation for generated project.
@objc(LoopbackDevice)
final class LoopbackDevice: NSObject {}
SWIFT

cat <<'SWIFT' > "$OUTPUT_DIR/HostApp/HostApp.swift"
import SwiftUI

@main
struct LoopbackDriverHostApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    var body: some View {
        Text("Loopback Driver Host App")
            .padding()
    }
}
SWIFT

cat <<'ENT' > "$OUTPUT_DIR/HostApp/LoopbackDriverApp.entitlements"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.developer.driverkit</key>
  <true/>
</dict>
</plist>
ENT

echo "==> Building Rust mixer dylib"
cargo build --release --manifest-path "$PROJECT_ROOT/Cargo.toml"

cp "$RUST_TARGET" "$OUTPUT_DIR/DriverExtension/Frameworks/libdevice_kit.dylib"

pushd "$OUTPUT_DIR" >/dev/null
xcodegen generate
popd >/dev/null

echo "==> Ready."
echo "Open the generated Xcode project at:"
echo "  $OUTPUT_DIR/LoopbackDriver.xcodeproj"

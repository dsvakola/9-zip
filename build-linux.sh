#!/usr/bin/env bash
# Builds 9-zip for Linux with pnpm and puts the packages in the Release folder.
# Needs Rust, Node.js, pnpm and the WebKitGTK build packages
# (on Ubuntu: libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev patchelf).
set -euo pipefail
cd "$(dirname "$0")"

pnpm install
pnpm tauri build --bundles deb,rpm,appimage

mkdir -p Release
cp src-tauri/target/release/bundle/deb/*.deb Release/
cp src-tauri/target/release/bundle/rpm/*.rpm Release/
cp src-tauri/target/release/bundle/appimage/*.AppImage Release/
echo "Done. The packages are in the Release folder:"
ls -1 Release

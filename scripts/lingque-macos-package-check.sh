#!/bin/bash
# Verify final artifacts; NEVER re-sign the .app after its updater archive exists.
set -euo pipefail
app='target/release/bundle/macos/Nuphus Workbench.app'
codesign --verify --deep --strict "$app"
lipo "$app/Contents/MacOS/nuphus-workbench" -verify_arch arm64
test -x "$app/Contents/MacOS/nuphus-workbench-mcp"
lipo "$app/Contents/MacOS/nuphus-workbench-mcp" -verify_arch arm64
scratch=$(mktemp -d)
mkdir "$scratch/archive" "$scratch/mount"
archives=(target/release/bundle/macos/*.app.tar.gz)
images=(target/release/bundle/dmg/*.dmg)
test "${#archives[@]}" -eq 1
test "${#images[@]}" -eq 1
tar -xzf "${archives[0]}" -C "$scratch/archive"
codesign --verify --deep --strict "$scratch/archive/Nuphus Workbench.app"
diff -qr "$app" "$scratch/archive/Nuphus Workbench.app"
hdiutil verify "${images[0]}"
hdiutil attach -readonly -nobrowse -mountpoint "$scratch/mount" "${images[0]}"
trap 'hdiutil detach "$scratch/mount" >/dev/null || true' EXIT
codesign --verify --deep --strict "$scratch/mount/Nuphus Workbench.app"
diff -qr "$app" "$scratch/mount/Nuphus Workbench.app"
hdiutil detach "$scratch/mount"
trap - EXIT
WORKBENCH_TEST_EXPECT_AUTH_REQUIRED=1 WORKBENCH_TEST_ROOT="$scratch/mcp" WORKBENCH_TEST_EXE="$app/Contents/MacOS/nuphus-workbench" WORKBENCH_TEST_MCP="$app/Contents/MacOS/nuphus-workbench-mcp" node scripts/workbench-mcp-smoke.mjs
mkdir "$scratch/launch"
NUPHUS_WORKBENCH_DATA_DIR="$scratch/launch" NUPHUS_WORKBENCH_PORT=47739 "$app/Contents/MacOS/nuphus-workbench" --background > "$scratch/launch.log" 2>&1 &
app_pid=$!
trap 'kill "$app_pid" 2>/dev/null || true' EXIT
ready=false
for attempt in $(seq 1 30); do
  if [ "$(curl --silent --output /dev/null --write-out '%{http_code}' http://127.0.0.1:47739/api/v1/discover || true)" = 200 ]; then
    ready=true
    break
  fi
  kill -0 "$app_pid"
  sleep 1
done
if [ "$ready" != true ]; then tail -40 "$scratch/launch.log"; exit 1; fi
kill "$app_pid"
wait "$app_pid" || true
trap - EXIT
echo 'ARM64 ad-hoc signature verified. Apple notarization and real Mac trackpad/Finder acceptance remain manual.' >> "$GITHUB_STEP_SUMMARY"

# Spec: macOS CEF Build & Package Flow

**Status:** Draft — post-refactor (agentmux-srv / agentmux-wsh rename, Tauri removed)

---

## Current State (post-refactor)

| Task | Status |
|------|--------|
| `cef:build:darwin` | ✓ Builds `agentmux-cef` → `dist/cef/` |
| `cef:bundle:darwin` | ✗ Stub — `echo "macOS CEF bundling not yet implemented"` |
| `cef:run:darwin` | ✓ Runs `dist/cef/agentmux-cef --url=http://localhost:5173` |
| `cef:dev` | Partial — build+bundle+serve, but bundle is a no-op on macOS |
| `package:macos` | ✗ Stub — `echo "macOS CEF packaging not yet implemented"` |
| `cef:package:portable` | ✗ Windows-only |

**Renamed crates (critical for path names):**
- `agentmuxsrv-rs` → `agentmux-srv` → builds to `dist/bin/agentmux-srv-{VERSION}-darwin.arm64`
- `wsh-rs` → `agentmux-wsh` → builds to `dist/bin/wsh-{VERSION}-darwin.arm64`

---

## How the CEF Host Finds Its Dependencies

Understanding `main.rs` and `sidecar.rs` before touching anything:

### 1. CEF Framework (macOS-specific)

```rust
// main.rs ~line 76
let loader = library_loader::LibraryLoader::new(&std::env::current_exe().unwrap(), false);
assert!(loader.load(), "Failed to load CEF framework");
```

`library_loader` is part of the `cef` crate. On macOS it looks for the framework relative to the executable — specifically in `{exe_dir}/Frameworks/Chromium Embedded Framework.framework/`. This is the path we must populate.

### 2. CEF Resources (pak files, locales)

```rust
// main.rs — resource path resolution
let runtime_dir = exe_dir.join("runtime");
let base_dir = if runtime_dir.exists() { runtime_dir } else { exe_dir.clone() };
let resources_dir = CefString::from(base_dir.to_str()); // .pak files go here
let locales_dir   = CefString::from(base_dir.join("locales").to_str());
```

**Implication:** For `dist/cef/`, there is no `runtime/` subdir, so `base_dir = exe_dir = dist/cef/`. Pak files go directly in `dist/cef/`, locales in `dist/cef/locales/`.

For a `.app` bundle where the exe is at `Contents/MacOS/agentmux-cef`: if we put resources in `Contents/MacOS/runtime/`, they will be found. If we use `Contents/Resources/`, they won't (code doesn't look there). **Use `Contents/MacOS/runtime/` as the resource base inside the `.app`.**

### 3. Backend Sidecar (agentmux-srv)

`sidecar.rs::resolve_backend_binary()` searches in order:

1. `{exe_dir}/agentmux-srv-{VERSION}-darwin.arm64` — **versioned, same dir** (portable/bundled)
2. `{exe_dir}/agentmux-srv` — plain, dev mode
3. `dist/bin/agentmux-srv-{VERSION}-darwin.arm64` — workspace dist/bin (reached from `dist/cef/` via `../../dist/bin`)

For bundled distribution, we must copy the versioned binary: `dist/cef/agentmux-srv-{VERSION}-darwin.arm64`.

### 4. wsh Binary

`sidecar.rs::deploy_wsh()` looks for `{exe_dir}/wsh` (plain name, no version suffix). It then copies it to `~/.config/agentmux-{version}/bin/wsh-{version}-darwin.arm64` at runtime.

We must copy `dist/bin/wsh-{VERSION}-darwin.arm64` → `dist/cef/wsh` (rename to plain `wsh`).

---

## Target Layouts

### Dev / `task cef:dev` (`dist/cef/`)

```
dist/cef/
├── agentmux-cef                                    # CEF host binary
├── agentmux-srv-{VERSION}-darwin.arm64             # backend sidecar (versioned)
├── wsh                                             # wsh binary (plain name)
├── Frameworks/
│   └── Chromium Embedded Framework.framework/      # CEF framework (library_loader finds it here)
│       ├── Chromium Embedded Framework             # main dylib
│       ├── Libraries/
│       └── Resources/
├── *.pak                                           # CEF resource files (flat, base_dir = exe_dir)
└── locales/
    └── en-US.pak
```

Note: `frontend/` is NOT needed in dev mode — CEF connects to the Vite dev server.

### Release (`.app` bundle → DMG)

```
AgentMux.app/
└── Contents/
    ├── Info.plist
    ├── MacOS/
    │   ├── agentmux-cef                            # CEF host (CFBundleExecutable)
    │   ├── Frameworks/                             # library_loader looks here
    │   │   └── Chromium Embedded Framework.framework/
    │   └── runtime/                               # base_dir (runtime/ exists → used as resource root)
    │       ├── agentmux-srv-{VERSION}-darwin.arm64
    │       ├── wsh
    │       ├── *.pak
    │       └── locales/
    │           └── en-US.pak
    └── Resources/
        ├── AgentMux.icns
        └── frontend/                              # built frontend bundle
```

Wait — the exe is at `Contents/MacOS/agentmux-cef`, so `exe_dir = Contents/MacOS/`. `library_loader` looks in `Contents/MacOS/Frameworks/`. The resource fallback checks `Contents/MacOS/runtime/` first; if it exists, that's where pak files go. Sidecar binary also goes in `Contents/MacOS/runtime/` so it's found at the versioned path.

Frontend path: the IPC server serves frontend from disk. Check `agentmux-cef/src/commands/window.rs` or `ipc.rs` for how the URL for the built frontend is resolved — it may need to point to `../Resources/frontend` relative to the exe.

---

## Implementation Plan

### Step 1 — Verify CEF framework location in build output

After `cargo build --release -p agentmux-cef`:

```bash
find target -type d -name "Chromium Embedded Framework.framework" 2>/dev/null
```

Confirm the framework exists before writing the bundle task. Expected location:
`target/release/build/cef-dll-sys-<hash>/out/Chromium Embedded Framework.framework/`

Also verify rpath on the binary:
```bash
otool -l target/release/agentmux-cef | grep -A2 LC_RPATH
```

If `@loader_path/Frameworks` is not present, add it in `agentmux-cef/build.rs`:
```rust
println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path/Frameworks");
```

### Step 2 — Implement `cef:bundle:darwin` in Taskfile.yml

Replace the stub at line ~475:

```yaml
cef:bundle:darwin:
    internal: true
    platforms: [darwin]
    cmds:
      - |
        VERSION=$(node -p "require('./package.json').version")

        # 1. Locate CEF framework in build output
        CEF_FW=$(find target -type d -name "Chromium Embedded Framework.framework" \
          -path "*/cef-dll-sys*/out/*" 2>/dev/null | head -1)
        if [ -z "$CEF_FW" ]; then
          echo "❌ Chromium Embedded Framework.framework not found — run cef:build first"
          exit 1
        fi
        echo "Found CEF framework: $CEF_FW"

        # 2. Copy framework to dist/cef/Frameworks/
        mkdir -p dist/cef/Frameworks
        rsync -a --delete "$CEF_FW" dist/cef/Frameworks/

        # 3. Strip non-en-US locales from framework (~15 MB saved)
        find "dist/cef/Frameworks/Chromium Embedded Framework.framework/Resources/locales" \
          -name "*.pak" ! -name "en-US.pak" -delete 2>/dev/null || true

        # 4. Copy pak files and locales flat into dist/cef/ (base_dir = exe_dir, no runtime/ subdir in dev)
        CEF_RESOURCES="$CEF_FW/Resources"
        cp -f "$CEF_RESOURCES"/*.pak dist/cef/ 2>/dev/null || true
        cp -f "$CEF_RESOURCES/icudtl.dat" dist/cef/ 2>/dev/null || true
        cp -f "$CEF_RESOURCES/v8_context_snapshot.bin" dist/cef/ 2>/dev/null || true
        mkdir -p dist/cef/locales
        cp -f "$CEF_RESOURCES/locales/en-US.pak" dist/cef/locales/ 2>/dev/null || true

        # 5. Copy versioned backend sidecar (sidecar.rs resolution order: versioned in exe_dir first)
        ARCH="arm64"
        cp "dist/bin/agentmux-srv-${VERSION}-darwin.${ARCH}" \
           "dist/cef/agentmux-srv-${VERSION}-darwin.${ARCH}"

        # 6. Copy wsh as plain name (deploy_wsh looks for {exe_dir}/wsh)
        cp "dist/bin/wsh-${VERSION}-darwin.${ARCH}" dist/cef/wsh
        chmod +x dist/cef/wsh

        echo "✓ Bundled macOS CEF runtime to dist/cef/"
```

### Step 3 — Confirm `task cef:dev` works end-to-end

```bash
task build:backend
task cef:build
task cef:bundle
# Then manually run to test before automating:
cd dist/cef && ./agentmux-cef --url=http://localhost:5173
```

Check stderr for:
- `"AgentMux CEF host starting"` — process started
- `"Backend ready: ws=..."` — `agentmux-srv` found and launched
- No `"Failed to load CEF framework"` — library_loader succeeded

### Step 4 — Implement `package:macos` in Taskfile.yml

Replace the stub at line ~73:

```yaml
package:macos:
    desc: Package the application for macOS (CEF .app + DMG). Outputs to ~/Desktop.
    platforms: [darwin]
    deps: [build:backend, build:frontend, cef:build, cef:bundle]
    cmds:
      - |
        VERSION=$(node -p "require('./package.json').version")
        APP="dist/AgentMux.app"
        MACOS="$APP/Contents/MacOS"
        RUNTIME="$MACOS/runtime"

        # Scaffold .app structure
        rm -rf "$APP"
        mkdir -p "$MACOS/Frameworks" "$RUNTIME/locales" "$APP/Contents/Resources/frontend"

        # CEF host binary (CFBundleExecutable)
        cp dist/cef/agentmux-cef "$MACOS/agentmux-cef"
        chmod +x "$MACOS/agentmux-cef"

        # CEF framework (library_loader looks in {exe_dir}/Frameworks/)
        rsync -a dist/cef/Frameworks/ "$MACOS/Frameworks/"

        # Resources in runtime/ (runtime_dir exists → base_dir = runtime/)
        ARCH="arm64"
        cp "dist/bin/agentmux-srv-${VERSION}-darwin.${ARCH}" \
           "$RUNTIME/agentmux-srv-${VERSION}-darwin.${ARCH}"
        cp "dist/bin/wsh-${VERSION}-darwin.${ARCH}" "$RUNTIME/wsh"
        chmod +x "$RUNTIME/agentmux-srv-${VERSION}-darwin.${ARCH}" "$RUNTIME/wsh"
        cp dist/cef/*.pak "$RUNTIME/" 2>/dev/null || true
        cp dist/cef/icudtl.dat "$RUNTIME/" 2>/dev/null || true
        cp dist/cef/v8_context_snapshot.bin "$RUNTIME/" 2>/dev/null || true
        cp dist/cef/locales/en-US.pak "$RUNTIME/locales/"

        # Built frontend (served from disk in release mode)
        cp -r dist/frontend/. "$APP/Contents/Resources/frontend/"

        # App icon
        [ -f agentmux-cef/resources/AgentMux.icns ] && \
          cp agentmux-cef/resources/AgentMux.icns "$APP/Contents/Resources/AgentMux.icns"

        # Info.plist
        BUNDLE_ID="ai.agentmux.cef.v$(echo $VERSION | tr '.' '-')"
        cat > "$APP/Contents/Info.plist" << PLIST
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
            "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0"><dict>
          <key>CFBundleIdentifier</key>      <string>${BUNDLE_ID}</string>
          <key>CFBundleName</key>            <string>AgentMux</string>
          <key>CFBundleDisplayName</key>     <string>AgentMux</string>
          <key>CFBundleExecutable</key>      <string>agentmux-cef</string>
          <key>CFBundleVersion</key>         <string>${VERSION}</string>
          <key>CFBundleShortVersionString</key> <string>${VERSION}</string>
          <key>CFBundleIconFile</key>        <string>AgentMux</string>
          <key>LSMinimumSystemVersion</key>  <string>12.0</string>
          <key>NSHighResolutionCapable</key> <true/>
          <key>NSRequiresAquaSystemAppearance</key> <false/>
          <key>com.apple.security.cs.allow-jit</key> <true/>
          <key>com.apple.security.cs.allow-unsigned-executable-memory</key> <true/>
          <key>com.apple.security.cs.disable-library-validation</key> <true/>
        </dict></plist>
        PLIST

        echo "✓ Built AgentMux.app (CEF, v${VERSION})"

      - |
        VERSION=$(node -p "require('./package.json').version")
        DMG="dist/AgentMux-CEF_${VERSION}_aarch64.dmg"
        hdiutil create -volname "AgentMux ${VERSION}" \
          -srcfolder dist/AgentMux.app -ov -format UDZO "$DMG"
        cp "$DMG" ~/Desktop/
        echo "✓ DMG → ~/Desktop/AgentMux-CEF_${VERSION}_aarch64.dmg"
```

### Step 5 — Verify frontend URL in release mode

In `agentmux-cef/src/app.rs` or `commands/window.rs`, find where the app URL is set when not given `--url=`. It must resolve the frontend relative to the exe — in the `.app` layout, this should be `../Resources/frontend/index.html` (from `Contents/MacOS/` up to `Contents/`, then into `Resources/`).

Check and fix if needed — this is likely already handled or uses a `file://` URL constructed from `exe_dir`.

---

## Open Questions (resolve before coding)

1. **CEF framework path for `library_loader`** — Confirm exactly where `LibraryLoader::new(exe, false).load()` looks on macOS. Run a quick test after `cef:build`: place a dummy framework dir at `dist/cef/Frameworks/Chromium Embedded Framework.framework/` and see if it loads. The cef crate source in `~/.cargo/registry/` has the `library_loader` implementation.

2. **Pak files location in framework** — On macOS, CEF pak files may be inside `Chromium Embedded Framework.framework/Resources/` rather than a separate directory. Verify after first build: `ls target/release/build/cef-dll-sys-*/out/Chromium\ Embedded\ Framework.framework/Resources/`.

3. **Frontend URL in production** — How does the built binary serve/load `frontend/`? Check `agentmux-cef/src/app.rs` around where the initial URL is constructed. If it uses `--url=` flag only and has no file:// fallback, add one pointing to `exe_dir/../Resources/frontend/index.html`.

4. **App icon** — `agentmux-cef/resources/` currently only has a Windows `.ico`. Need `.icns` for macOS. Can convert from existing icon source or use Tauri's `src-tauri/icons/icon.icns` as a stopgap (Tauri source files may still be around in git history).

---

## Files to Change

| File | Change |
|------|--------|
| `Taskfile.yml` | Replace `cef:bundle:darwin` stub (~line 475); replace `package:macos` stub (~line 73) |
| `agentmux-cef/build.rs` | Add `@loader_path/Frameworks` rpath if `otool` shows it's missing |
| `agentmux-cef/src/app.rs` | Add file:// frontend URL fallback for production mode if missing |
| `agentmux-cef/resources/` | Add `AgentMux.icns` for macOS app icon |

---

## Out of Scope

- Code signing / notarization
- Universal binary (x86_64 + arm64) — arm64 only for now
- Auto-update (Sparkle)

#!/usr/bin/env bash
# AppImage AppRun for AgentMux on Linux (CEF runtime).
#
# Design notes:
#   - Earlier versions set WEBKIT_DISABLE_DMABUF_RENDERER, XMODIFIERS,
#     GTK_IM_MODULE, GDK_BACKEND. Those were Tauri/WebKitGTK workarounds.
#     CEF doesn't read WebKit env vars, has its own IME path
#     (InputMethodAuralinux), and uses Ozone (not GDK) for Wayland/X11.
#     Carrying them forward biased CEF toward stale Tauri-era behavior.
#     Audit + rationale: docs/specs/linux-cef-flags-audit-2026-05-08.md.
#
#   - **Extract-once-cache (Phase 2 of cold-launch tax fix).** When the
#     AppImage is launched for the first time, its SquashFS gets mounted
#     via FUSE and every file read decompresses on demand → ~3s cold
#     start. This script extracts the contents to
#     $HOME/.local/share/agentmux/extracted/<VERSION>/ on first run, then
#     re-execs from there. Subsequent launches see the cache and skip
#     extraction → ~1s warm start. Spec:
#     docs/specs/linux-appimage-cold-launch-tax-2026-05-08.md (Phase 2).
#
#   - Icon / desktop registration: the agentmux-cef binary sets
#     xdg_toplevel.app_id="agentmux"; this script registers a matching
#     ~/.local/share/applications/agentmux.desktop via
#     install-linux-desktop.sh.
set -e
this_dir="$(readlink -f "$(dirname "$0")")"

# ---- run_normally: shared body for both "ran from extract dir" and
# ---- "ran from FUSE mount as fallback". Sets env, registers desktop file,
# ---- exec's the host binary. ------------------------------------------
run_normally() {
    export APPDIR="$this_dir"
    if [ -n "$APPIMAGE" ] && [ -x "$this_dir/install-linux-desktop.sh" ]; then
        bash "$this_dir/install-linux-desktop.sh" "$APPIMAGE" || true
    fi
    # libcef.so + EGL/GLESv2 sit in usr/bin alongside agentmux-cef. Binary
    # is built without RPATH so we set LD_LIBRARY_PATH explicitly.
    export LD_LIBRARY_PATH="$this_dir/usr/bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    exec "$this_dir/usr/bin/agentmux" "$@"
}

# ---- Detect a re-exec from the cache so we don't loop. ----------------
VERSION="$(cat "$this_dir/usr/share/agentmux/VERSION" 2>/dev/null || echo unknown)"
EXTRACT_DIR="$HOME/.local/share/agentmux/extracted/$VERSION"

if [ "${AGENTMUX_EXTRACTED_RUN:-0}" = "1" ] || [ "$this_dir" = "$EXTRACT_DIR" ]; then
    # We're already running from the extracted cache (or marked as such).
    # Just run the host binary; no extraction work to do.
    run_normally "$@"
fi

# ---- First run on a FUSE mount. Try to extract to disk. ---------------
# If extraction succeeds, we re-exec from the cached copy. If it fails for
# any reason (no $HOME, full disk, denied perms), fall through to running
# from the FUSE mount unchanged — slow but correct.
if [ ! -x "$EXTRACT_DIR/usr/bin/agentmux" ]; then
    if mkdir -p "$(dirname "$EXTRACT_DIR")" 2>/dev/null; then
        # Extract to a temp dir, then rename. If interrupted, the next run
        # sees a missing or partial $EXTRACT_DIR and retries; the final
        # destination is only created on full success.
        # PID-scoped temp so two simultaneous first-runs don't share state.
        TMP_DIR="${EXTRACT_DIR}.tmp.$$"
        rm -rf "$TMP_DIR" 2>/dev/null || true
        echo "[agentmux] First-run extraction of v${VERSION} → ${EXTRACT_DIR} (one-time, ~2-3s)" >&2
        if cp -a "$this_dir/." "$TMP_DIR/" 2>/dev/null; then
            # Concurrent-launch race: two simultaneous first-runs both
            # pass the existence check above. `mv -T` is strict rename —
            # it fails if target exists rather than nesting into it. We
            # tolerate that failure: the winning instance's $EXTRACT_DIR
            # is already valid for us to re-exec from. `set -e` at the
            # top of the script would otherwise abort the loser. (Codex
            # P2 round-2 on PR #788.)
            if mv -T "$TMP_DIR" "$EXTRACT_DIR" 2>/dev/null; then
                : # we won the race
            else
                echo "[agentmux] Cache populated by a concurrent instance; reusing" >&2
                rm -rf "$TMP_DIR" 2>/dev/null || true
            fi
            # Best-effort cleanup of older extractions. Keep the two most
            # recently modified version dirs (the current one plus the
            # immediately previous, in case the user is running both
            # concurrently). Fail silently — this is hygiene, not critical.
            (
                cd "$(dirname "$EXTRACT_DIR")" 2>/dev/null || exit 0
                ls -1t 2>/dev/null | tail -n +3 | while IFS= read -r old; do
                    [ -d "$old" ] && [ "$old" != "$VERSION" ] && rm -rf "$old"
                done
            ) || true
        else
            echo "[agentmux] Extraction failed; running from FUSE mount" >&2
            rm -rf "$TMP_DIR" 2>/dev/null || true
        fi
    fi
fi

# If extraction succeeded, re-exec from the cached copy.
if [ -x "$EXTRACT_DIR/usr/bin/agentmux" ] && [ -x "$EXTRACT_DIR/AppRun" ]; then
    export AGENTMUX_EXTRACTED_RUN=1
    exec "$EXTRACT_DIR/AppRun" "$@"
fi

# Fallback: extraction unavailable, run from FUSE mount.
run_normally "$@"

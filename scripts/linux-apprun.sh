#!/usr/bin/env bash
# AppImage AppRun for AgentMux on Linux (CEF runtime).
#
# Design notes:
#   - Earlier versions set WEBKIT_DISABLE_DMABUF_RENDERER, XMODIFIERS,
#     GTK_IM_MODULE, GDK_BACKEND. Those were Tauri/WebKitGTK workarounds.
#     CEF doesn't read WebKit env vars, has its own IME path
#     (InputMethodAuralinux), and uses Ozone (not GDK) for Wayland/X11.
#     Carrying them forward biased CEF toward stale Tauri-era behavior.
#     Audit + rationale: docs/specs/archive/linux-cef-flags-audit-2026-05-08.md.
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
#     xdg_toplevel.app_id="agentmux-<channel>-<version>" (see
#     window_settings.rs::linux_app_id()); this script computes the same
#     string from the CHANNEL/VERSION markers staged alongside the binary
#     and registers a matching ~/.local/share/applications/<app_id>.desktop
#     via install-linux-desktop.sh. One file per app_id keeps two
#     differently-versioned/channeled instances from overwriting each
#     other's desktop entry (and thus dock icon).
set -e
this_dir="$(readlink -f "$(dirname "$0")")"

# ---- run_normally: shared body for both "ran from extract dir" and
# ---- "ran from FUSE mount as fallback". Sets env, registers desktop file,
# ---- exec's the LAUNCHER (which then supervises srv + the CEF host).
# ----
# ---- Exec target changed in A0 (SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH
# ---- 2026_06_05 §3). Previously this execed `usr/bin/agentmux` directly,
# ---- which left the launcher's window/pool/instance reducer and saga
# ---- coordinator dormant (host's launcher_ipc::connect_to_launcher
# ---- returned None on non-Windows). Now AppRun → launcher → host so the
# ---- supervision tree exists; A1 (a follow-up PR) implements the Unix
# ---- IPC server that drives the reducer.
run_normally() {
    export APPDIR="$this_dir"
    if [ -n "$APPIMAGE" ] && [ -x "$this_dir/install-linux-desktop.sh" ]; then
        # Same resolution order as agentmux_common::DataPaths for
        # Installed/Portable modes: an explicit AGENTMUX_CHANNEL override
        # wins, else the channel baked into this build at compile time.
        CHANNEL="${AGENTMUX_CHANNEL:-$(cat "$this_dir/usr/share/agentmux/CHANNEL" 2>/dev/null || echo stable)}"
        APP_ID="agentmux-${CHANNEL}-${VERSION}"
        bash "$this_dir/install-linux-desktop.sh" "$APPIMAGE" "$APP_ID" || true
    fi
    # libcef.so + EGL/GLESv2 sit in usr/bin alongside agentmux-cef. Binary
    # is built without RPATH so we set LD_LIBRARY_PATH explicitly.
    export LD_LIBRARY_PATH="$this_dir/usr/bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    # What a start-at-login entry should run: the AppImage file itself, not
    # this extract-cache dir (pruned on update). The launcher reads this once
    # and removes it from its env (SPEC_START_WITH_OS_2026_09_25.md §3.1).
    if [ -n "$APPIMAGE" ]; then
        export AGENTMUX_STABLE_EXE="$APPIMAGE"
    fi
    exec "$this_dir/usr/bin/agentmux-launcher" "$@"
}

# ---- Detect a re-exec from the cache so we don't loop. ----------------
VERSION="$(cat "$this_dir/usr/share/agentmux/VERSION" 2>/dev/null || echo unknown)"
# Key the cache on the BUILD, not the version. `task package` deliberately
# does not bump the version, so every local build of a version used to share
# one extraction dir — and because the guard below is "does a launcher already
# exist there", the FIRST build extracted won permanently: later builds
# silently re-exec'd the older binary, along with the per-build data-dir
# channel baked into it. Two local 0.56.3 builds reproduced exactly that; the
# second launch ran the first's binary in the first's channel, so a fix that
# had just been built was never actually exercised.
# Release AppImages carry no BUILD_ID and keep one cache per version, as before.
BUILD_ID="$(cat "$this_dir/usr/share/agentmux/BUILD_ID" 2>/dev/null || true)"
CACHE_KEY="${BUILD_ID:-$VERSION}"
EXTRACT_DIR="$HOME/.local/share/agentmux/extracted/$CACHE_KEY"

# The marker carries the dir it refers to, and is honoured only when it names
# THIS dir. It used to be a bare "1", which the app then exported into every
# child process — including the shells in its own terminal panes. So launching
# any AppImage from inside a running AgentMux inherited it, took this branch,
# and ran from the FUSE mount without ever extracting or checking its own
# cache. Observed live: a freshly built AppImage launched from a pane ran
# mounted, no extraction, no message. A stale inherited value now names a
# different dir and is ignored.
if [ "${AGENTMUX_EXTRACTED_RUN:-}" = "$this_dir" ] || [ "$this_dir" = "$EXTRACT_DIR" ]; then
    # We're already running from the extracted cache (or marked as such).
    # Just run the host binary; no extraction work to do.
    run_normally "$@"
fi

# ---- First run on a FUSE mount. Try to extract to disk. ---------------
# If extraction succeeds, we re-exec from the cached copy. If it fails for
# any reason (no $HOME, full disk, denied perms), fall through to running
# from the FUSE mount unchanged — slow but correct.
if [ ! -x "$EXTRACT_DIR/usr/bin/agentmux-launcher" ]; then
    if mkdir -p "$(dirname "$EXTRACT_DIR")" 2>/dev/null; then
        # Extract to a temp dir, then rename. If interrupted, the next run
        # sees a missing or partial $EXTRACT_DIR and retries; the final
        # destination is only created on full success.
        # PID-scoped temp so two simultaneous first-runs don't share state.
        TMP_DIR="${EXTRACT_DIR}.tmp.$$"
        rm -rf "$TMP_DIR" 2>/dev/null || true
        echo "[agentmux] First-run extraction of ${CACHE_KEY} → ${EXTRACT_DIR} (one-time, ~2-3s)" >&2
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
            # recently modified dirs (the current one plus the immediately
            # previous, in case the user is running both concurrently).
            #
            # Now also skips any dir a LIVE process is executing from. That
            # guard did not matter while the key was the version — those dirs
            # changed only on release. Keyed per build they turn over on every
            # `task package`, so "keep 2" alone would eventually delete the
            # tree a running instance is still lazily reading its binaries,
            # .pak files and libcef.so out of, killing a session that was
            # doing nothing wrong. Each extraction is ~400 MB, so unbounded
            # growth is not an option either — hence prune, but never prune
            # something in use. Fail silently: this is hygiene, not critical.
            (
                base="$(dirname "$EXTRACT_DIR")"
                cd "$base" 2>/dev/null || exit 0
                in_use=" "
                for exe in /proc/[0-9]*/exe; do
                    tgt="$(readlink "$exe" 2>/dev/null)" || continue
                    case "$tgt" in
                        "$base"/*)
                            rest="${tgt#"$base"/}"
                            in_use="${in_use}${rest%%/*} "
                            ;;
                    esac
                done
                ls -1t 2>/dev/null | tail -n +3 | while IFS= read -r old; do
                    [ -d "$old" ] || continue
                    [ "$old" = "$CACHE_KEY" ] && continue
                    case "$in_use" in *" $old "*) continue ;; esac
                    rm -rf "$old"
                done
            ) || true
        else
            echo "[agentmux] Extraction failed; running from FUSE mount" >&2
            rm -rf "$TMP_DIR" 2>/dev/null || true
        fi
    fi
fi

# If extraction succeeded, re-exec from the cached copy.
if [ -x "$EXTRACT_DIR/usr/bin/agentmux-launcher" ] && [ -x "$EXTRACT_DIR/AppRun" ]; then
    export AGENTMUX_EXTRACTED_RUN="$EXTRACT_DIR"
    exec "$EXTRACT_DIR/AppRun" "$@"
fi

# Fallback: extraction unavailable, run from FUSE mount.
run_normally "$@"

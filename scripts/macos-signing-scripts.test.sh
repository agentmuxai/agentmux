#!/usr/bin/env bash
# macos-signing-scripts.test.sh — tests for the scripts the release's signing job runs on an app that came out of
# another job: unpack-macos-app.sh, sign-macos-app.sh, verify-srv-identity-darwin.sh, notarize-macos.sh and
# make-macos-dmg.sh.
#
# Two rules, both because the app (and the zip it came in) is input from code that ran with no credentials, while
# these scripts run with them:
#   1. the unpack step refuses a zip that writes outside its folder, a second top-level item, a non-regular file and a
#      symlink that leaves the app;
#   2. none of the scripts starts anything that is in the app: they sign, notarize, staple and pack it, and read its
#      signature. The signing steps run here against an app whose every file is a program that leaves a mark when it
#      is started, with the tools (codesign, security, xcrun, hdiutil, ditto, osascript) replaced by stand-ins that
#      only record what they were asked: no mark may appear.
#
# Runs anywhere (the tools are stand-ins; the unpack tests need ditto and zipinfo and are skipped without them).
# Usage: bash scripts/macos-signing-scripts.test.sh    (exit 0 = all pass)

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok()   { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad()  { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }
skip() { printf '  SKIP  %s\n' "$1"; }

SCRIPTS=(unpack-macos-app.sh sign-macos-app.sh verify-srv-identity-darwin.sh notarize-macos.sh make-macos-dmg.sh)

# ── 1. The unpack step ────────────────────────────────────────────────────────
if command -v ditto > /dev/null 2>&1 && command -v zipinfo > /dev/null 2>&1 && command -v python3 > /dev/null 2>&1; then
    unpack() { bash "$HERE/unpack-macos-app.sh" "$1" "$2" > "$TMP/unpack.out" 2>&1; }
    mkzip() { # <zip> <name>... : a zip with those entry names (empty files), written by python so any name goes in
        local zip="$1"; shift
        python3 -I - "$zip" "$@" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w") as z:
    for name in sys.argv[2:]:
        z.writestr(name, b"x")
PY
    }
    refuses() { # <label> <zip>
        local dest="$TMP/dest-$RANDOM"
        if unpack "$2" "$dest"; then bad "$1" "the zip was accepted"; else ok "$1"; fi
        [ ! -e "$dest/AgentMux.app" ] || bad "$1 (nothing left unpacked)" "AgentMux.app remains"
    }

    # A good app: the layout's relative links, a framework with its Versions/Current chain.
    good="$TMP/good/AgentMux.app"
    mkdir -p "$good/Contents/MacOS" "$good/Contents/Helpers/S.app/Contents/MacOS" "$good/Contents/Resources/frontend" \
        "$good/Contents/Frameworks/F.framework/Versions/A"
    echo '<plist/>' > "$good/Contents/Info.plist"
    : > "$good/Contents/Helpers/S.app/Contents/MacOS/agentmux-srv"
    ln -s "../Helpers/S.app/Contents/MacOS/agentmux-srv" "$good/Contents/MacOS/agentmux-srv-1"
    ln -s "../Resources/frontend" "$good/Contents/MacOS/frontend"
    ln -s A "$good/Contents/Frameworks/F.framework/Versions/Current"
    (cd "$TMP/good" && ditto -c -k --keepParent --sequesterRsrc AgentMux.app "$TMP/good.zip")
    if unpack "$TMP/good.zip" "$TMP/dest-good" && [ -L "$TMP/dest-good/AgentMux.app/Contents/MacOS/frontend" ]; then
        ok "a good app unpacks with its relative symlinks"
    else
        bad "a good app unpacks with its relative symlinks" "$(cat "$TMP/unpack.out")"
    fi

    mkzip "$TMP/dotdot.zip" "AgentMux.app/Contents/Info.plist" "../evil"
    refuses "refuses an entry with .. at the start" "$TMP/dotdot.zip"
    mkzip "$TMP/mid.zip" "AgentMux.app/Contents/Info.plist" "AgentMux.app/Contents/../../evil"
    refuses "refuses an entry with .. in the middle" "$TMP/mid.zip"
    mkzip "$TMP/abs.zip" "AgentMux.app/Contents/Info.plist" "/tmp/evil"
    refuses "refuses an absolute entry" "$TMP/abs.zip"
    mkzip "$TMP/other.zip" "AgentMux.app/Contents/Info.plist" "evil.sh"
    refuses "refuses a second top-level item" "$TMP/other.zip"
    mkzip "$TMP/prefix.zip" "AgentMux.app/Contents/Info.plist" "AgentMux.app.evil/x"
    refuses "refuses a folder that only starts like AgentMux.app" "$TMP/prefix.zip"
    mkzip "$TMP/bs.zip" "AgentMux.app/Contents/Info.plist" 'AgentMux.app\..\evil'
    refuses "refuses an entry with a backslash" "$TMP/bs.zip"
    mkzip "$TMP/noplist.zip" "AgentMux.app/Contents/MacOS/x"
    refuses "refuses an app with no Info.plist" "$TMP/noplist.zip"

    # Entries whose names are clean but whose records are not: a file written through a symlink the same zip made, a
    # duplicate, a special file. The check is on the zip's own records, so nothing may be written outside the destination.
    mkrec() { # <zip> <kind>
        python3 -I - "$1" "$2" "$TMP/outside-target" <<'PY'
import sys, zipfile
zip_path, kind, outside = sys.argv[1:4]
def entry(z, name, data=b"x", mode=0o100644):
    zi = zipfile.ZipInfo(name); zi.external_attr = mode << 16; z.writestr(zi, data)
with zipfile.ZipFile(zip_path, "w") as z:
    entry(z, "AgentMux.app/Contents/Info.plist")
    if kind == "through-link":
        entry(z, "AgentMux.app/Contents/l", outside.encode(), 0o120777)
        entry(z, "AgentMux.app/Contents/l/x")
    elif kind == "through-link-first":
        entry(z, "AgentMux.app/Contents/l/x")
        entry(z, "AgentMux.app/Contents/l", outside.encode(), 0o120777)
    elif kind == "duplicate":
        entry(z, "AgentMux.app/Contents/a"); entry(z, "AgentMux.app/Contents/a", b"y")
    elif kind == "fifo":
        entry(z, "AgentMux.app/Contents/pipe", b"", 0o010644)
PY
    }
    for kind in through-link through-link-first duplicate fifo; do
        mkdir -p "$TMP/outside-target"
        mkrec "$TMP/rec-$kind.zip" "$kind"
        refuses "refuses a zip with a $kind record" "$TMP/rec-$kind.zip"
        if [ -z "$(ls -A "$TMP/outside-target")" ]; then ok "nothing was written outside the destination ($kind)"
        else bad "nothing was written outside the destination ($kind)" "$(ls -A "$TMP/outside-target")"; fi
    done

    # Symlinks that leave: absolute, relative past the top, and a chain whose first hop stays inside.
    for kind in absolute relative chain; do
        bd="$TMP/link-$kind/AgentMux.app"
        mkdir -p "$bd/Contents/MacOS"
        echo '<plist/>' > "$bd/Contents/Info.plist"
        case "$kind" in
            absolute) ln -s /etc "$bd/Contents/out" ;;
            relative) ln -s ../../../../outside "$bd/Contents/out" ;;
            chain) ln -s ../.. "$bd/Contents/hop"; ln -s hop/../../outside "$bd/Contents/out" ;;
        esac
        (cd "$TMP/link-$kind" && ditto -c -k --keepParent --sequesterRsrc AgentMux.app "$TMP/link-$kind.zip")
        refuses "refuses a $kind symlink that leaves the app" "$TMP/link-$kind.zip"
    done
else
    skip "unpack tests (need ditto, zipinfo and python3)"
fi

# ── 2. Nothing in the app is started ──────────────────────────────────────────
STUBS="$TMP/stubs"; mkdir -p "$STUBS"
CANARY="$TMP/canary"
RECORD="$TMP/record"; : > "$RECORD"
for tool in codesign security xcrun hdiutil osascript strip sips; do
    cat > "$STUBS/$tool" <<'STUB'
#!/usr/bin/env bash
echo "$(basename "$0") $*" >> "$RECORD"
case "$(basename "$0")" in
    codesign)
        # codesign -d -r- <path>: the designated requirement the srv identity check reads.
        case " $* " in *" -d "*) echo "designated => identifier \"ai.agentmux.srv\" and anchor apple generic and certificate leaf[subject.OU] = \"ABCDE12345\"" ;; esac ;;
    xcrun)
        case "$*" in *"notarytool submit"*) echo "  status: Accepted" ;; esac ;;
    hdiutil)
        # create <...> <out>, convert <in> -o <out>: leave a file where the script expects one.
        case "$1" in
            create) for last; do :; done; : > "$last" ;;
            convert) while [ $# -gt 0 ]; do [ "$1" = "-o" ] && { : > "$2"; break; }; shift; done ;;
        esac ;;
esac
exit 0
STUB
    chmod +x "$STUBS/$tool"
done
# ditto: copy (the app into the DMG stage) or make an empty zip; both just move bytes, as the real one does.
cat > "$STUBS/ditto" <<'STUB'
#!/usr/bin/env bash
echo "ditto $*" >> "$RECORD"
if [ "$1" = "-c" ]; then for last; do :; done; : > "$last"; else cp -R "$1" "$2"; fi
STUB
chmod +x "$STUBS/ditto"

# An app whose every file is a program that leaves a mark when it is started.
app="$TMP/canary-app/AgentMux.app"
plant() { mkdir -p "$(dirname "$1")"; printf '#!/bin/sh\ntouch "%s"\n' "$CANARY" > "$1"; chmod +x "$1"; }
plant "$app/Contents/MacOS/agentmux-cef"
plant "$app/Contents/MacOS/agentmux-launcher"
plant "$app/Contents/MacOS/libGLESv2.dylib"
plant "$app/Contents/MacOS/tools/remote/aarch64-apple-darwin/agentmux-remote"
plant "$app/Contents/Helpers/AgentMux Server.app/Contents/MacOS/agentmux-srv"
plant "$app/Contents/Helpers/AgentMux MCP.app/Contents/MacOS/agentmux-mcp"
plant "$app/Contents/Helpers/AgentMux Shell Wrapper.app/Contents/MacOS/agentmux-bashwrap"
for h in "AgentMux Helper" "AgentMux Helper (GPU)" "AgentMux Helper (Renderer)"; do
    plant "$app/Contents/Frameworks/$h.app/Contents/MacOS/$h"
    plant "$app/Contents/Frameworks/$h.app/Contents/MacOS/libEGL.dylib"
done
plant "$app/Contents/Frameworks/Chromium Embedded Framework.framework/Libraries/libvk_swiftshader.dylib"
plant "$app/Contents/Frameworks/Chromium Embedded Framework.framework/Versions/A/Chromium Embedded Framework"
echo '<plist/>' > "$app/Contents/Info.plist"
ENT="$TMP/entitlements.plist"; echo '<plist/>' > "$ENT"

run_with_stubs() { PATH="$STUBS:$PATH" RECORD="$RECORD" EXPECTED_TEAM_ID=ABCDE12345 "$@" > "$TMP/run.out" 2>&1; }

if run_with_stubs bash "$HERE/sign-macos-app.sh" "$app" "$ENT" "Test Cert" && [ ! -e "$CANARY" ]; then
    ok "signing the app starts nothing in it"
else
    bad "signing the app starts nothing in it" "exit or mark: $(tail -3 "$TMP/run.out") $([ -e "$CANARY" ] && echo 'A PROGRAM IN THE APP WAS STARTED')"
fi
grep -q 'ai.agentmux.srv' "$RECORD" && ok "the stand-in codesign was asked to sign srv under its fixed identifier" \
    || bad "the stand-in codesign was asked to sign srv under its fixed identifier" "$(head -3 "$RECORD")"

if run_with_stubs bash "$HERE/notarize-macos.sh" app "$app" && [ ! -e "$CANARY" ]; then
    ok "notarizing and stapling the app starts nothing in it"
else
    bad "notarizing and stapling the app starts nothing in it" "$(tail -3 "$TMP/run.out")"
fi
grep -q 'stapler staple' "$RECORD" && grep -q 'stapler validate' "$RECORD" \
    && ok "the app is stapled and the staple is validated" || bad "the app is stapled and the staple is validated" "$(cat "$RECORD" | tail -5)"

if run_with_stubs bash "$HERE/make-macos-dmg.sh" "$app" "$TMP/out/AgentMux_0.0.0_arm64.dmg" && [ ! -e "$CANARY" ]; then
    ok "packing the app into the DMG starts nothing in it"
else
    bad "packing the app into the DMG starts nothing in it" "$(tail -3 "$TMP/run.out")"
fi

# A refused notarization is exit 2 and nothing is stapled.
: > "$RECORD"
cat > "$STUBS/xcrun" <<'STUB'
#!/usr/bin/env bash
echo "xcrun $*" >> "$RECORD"
case "$*" in *"notarytool submit"*) echo "  status: Invalid" ;; esac
exit 0
STUB
if PATH="$STUBS:$PATH" RECORD="$RECORD" bash "$HERE/notarize-macos.sh" app "$app" > "$TMP/run.out" 2>&1; then
    bad "a notarization Apple does not accept fails" "exit 0"
elif [ "$?" -eq 2 ] && ! grep -q 'stapler' "$RECORD"; then
    ok "a notarization Apple does not accept fails (exit 2) and staples nothing"
else
    bad "a notarization Apple does not accept fails (exit 2) and staples nothing" "$(cat "$RECORD")"
fi

# ── 3. No command in these scripts is a path in the app ───────────────────────
# The marks above catch a run; this catches the line that would start one: a script, `source`, `exec`, `open` or a
# bare path in the command position whose first word is a variable that holds a path in the app.
APPVARS='APP|FW|SRV_APP_BIN|MCP_APP_BIN|BASHWRAP_APP_BIN|ha|ta|helper|dy|TARGETS'
offending=""
for s in "${SCRIPTS[@]}"; do
    hit="$(grep -nE "^[[:space:]]*(\"?\\\$\{?($APPVARS)\}?|(bash|sh|source|\\.|exec|eval|open|xargs|python3?|node)[[:space:]]+\"?\\\$\{?($APPVARS)\}?)" "$HERE/$s" \
        | grep -vE '^[0-9]+:[[:space:]]*#' | grep -vE '^[0-9]+:[[:space:]]*"\$\{SIGN\[@\]\}"' || true)"
    [ -z "$hit" ] || offending="$offending $s: $hit"
done
if [ -z "$offending" ]; then
    ok "no script has a command that is a path in the app"
else
    bad "no script has a command that is a path in the app" "$offending"
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]

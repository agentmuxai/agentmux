#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# ci-install-go-task.test.sh — tests for ci-install-go-task.sh and ci-require-tools.sh. A fake `curl` serves a
# fake release from a folder, so nothing is downloaded and no pause is real.
#
# Usage: bash scripts/ci-install-go-task.test.sh   (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL="$HERE/ci-install-go-task.sh"
REQUIRE="$HERE/ci-require-tools.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
BIN="$TMP/bin"; REL="$TMP/release"; STUB="$TMP/stub"
mkdir -p "$BIN" "$REL" "$STUB"
pass=0; fail=0
ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }
check() { # name, condition result (0 = pass), detail
    if [ "$2" -eq 0 ]; then ok "$1"; else bad "$1" "${3:-}"; fi
}
sha() { sha256sum < "$1" | cut -d' ' -f1; }

# A fake release: task_linux_amd64.tar.gz holding a `task` that prints the version it was built with.
make_release() { # version
    local d="$TMP/pkg"; rm -rf "$d"; mkdir -p "$d"
    printf '#!/usr/bin/env bash\necho "Task version: v%s (h1:fake)"\n' "$1" > "$d/task"
    chmod +x "$d/task"
    tar -czf "$REL/task_linux_amd64.tar.gz" -C "$d" task
}
make_pins() { # version hash
    printf 'version %s\n%s  task_linux_amd64.tar.gz\n' "$1" "$2" > "$TMP/pins"
}

# curl serves $REL/<name> for the URL's file name; the first FAKE_CURL_FAILS calls fail like a 504.
cat > "$BIN/curl" <<'EOF'
#!/usr/bin/env bash
out=""; url=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output) out="$2"; shift 2 ;;
        --connect-timeout|--max-time) shift 2 ;;
        --*) shift ;;
        *) url="$1"; shift ;;
    esac
done
n=$(cat "$STUB_DIR/curl-calls" 2>/dev/null || echo 0)
echo $((n + 1)) > "$STUB_DIR/curl-calls"
echo "$url" >> "$STUB_DIR/curl-urls"
if [ "$n" -lt "${FAKE_CURL_FAILS:-0}" ]; then echo "curl: (22) The requested URL returned error: 504" >&2; exit 22; fi
cp "$REL_DIR/$(basename "$url")" "$out"
EOF
chmod +x "$BIN/curl"

run_install() { # extra env words..., runs the installer into $TMP/dest; sets OUT and RC
    rm -rf "$TMP/dest" "$STUB"/curl-calls "$STUB"/curl-urls; : > "$TMP/gh-path"
    OUT="$(env PATH="$BIN:$PATH" STUB_DIR="$STUB" REL_DIR="$REL" CI_GO_TASK_PINS="$TMP/pins" \
        CI_GO_TASK_PLATFORM=linux_amd64 CI_GO_TASK_PAUSE=0 GITHUB_PATH="$TMP/gh-path" "$@" \
        bash "$INSTALL" "$TMP/dest" 2>&1)"
    RC=$?
}
calls() { cat "$STUB/curl-calls" 2>/dev/null || echo 0; }

echo "ci-install-go-task.sh"
make_release 3.99.0; make_pins 3.99.0 "$(sha "$REL/task_linux_amd64.tar.gz")"

run_install
check "installs the pinned release and checks it runs" "$([ "$RC" -eq 0 ] && [ -x "$TMP/dest/task" ] && echo 0 || echo 1)" "rc=$RC $OUT"
check "downloads from the GitHub release URL of the pinned version" \
    "$(grep -q '^https://github.com/go-task/task/releases/download/v3.99.0/task_linux_amd64.tar.gz$' "$STUB/curl-urls" && echo 0 || echo 1)" "$(cat "$STUB/curl-urls")"
check "puts the install dir on the later steps' PATH" "$(grep -qx "$TMP/dest" "$TMP/gh-path" && echo 0 || echo 1)" "$(cat "$TMP/gh-path")"

run_install FAKE_CURL_FAILS=2
check "two failed downloads are retried and the third succeeds" "$([ "$RC" -eq 0 ] && [ "$(calls)" -eq 3 ] && echo 0 || echo 1)" "rc=$RC calls=$(calls) $OUT"

run_install FAKE_CURL_FAILS=9
check "a feed that never answers fails after three tries" "$([ "$RC" -ne 0 ] && [ "$(calls)" -eq 3 ] && echo 0 || echo 1)" "rc=$RC calls=$(calls)"
check "and says which file and how many tries" "$(echo "$OUT" | grep -q 'after 3 tries' && echo 0 || echo 1)" "$OUT"
check "and installs nothing" "$([ ! -e "$TMP/dest/task" ] && echo 0 || echo 1)"

make_pins 3.99.0 "0000000000000000000000000000000000000000000000000000000000000000"
run_install
check "an archive with another hash is refused, without retrying" "$([ "$RC" -ne 0 ] && [ "$(calls)" -eq 1 ] && echo 0 || echo 1)" "rc=$RC calls=$(calls)"
check "and the refusal names both hashes" "$(echo "$OUT" | grep -q 'is not the pinned release' && echo 0 || echo 1)" "$OUT"
check "and installs nothing (hash)" "$([ ! -e "$TMP/dest/task" ] && echo 0 || echo 1)"

make_pins 4.0.0 "$(sha "$REL/task_linux_amd64.tar.gz")"
run_install
check "a binary that reports another version than the pin fails" "$([ "$RC" -ne 0 ] && echo "$OUT" | grep -q 'expected version 4.0.0' && echo 0 || echo 1)" "rc=$RC $OUT"

make_pins 3.99.0 "$(sha "$REL/task_linux_amd64.tar.gz")"
run_install CI_GO_TASK_PLATFORM=linux_riscv64
check "a platform with no pin fails before any download" "$([ "$RC" -ne 0 ] && [ "$(calls)" -eq 0 ] && echo "$OUT" | grep -q 'no pinned hash' && echo 0 || echo 1)" "rc=$RC calls=$(calls) $OUT"

run_install CI_GO_TASK_PINS="$TMP/missing"
check "a missing pins file fails" "$([ "$RC" -ne 0 ] && echo "$OUT" | grep -q 'pins file not found' && echo 0 || echo 1)" "$OUT"

echo "the real pins file"
PIN_OK=0
for f in task_darwin_amd64.tar.gz task_darwin_arm64.tar.gz task_linux_amd64.tar.gz task_linux_arm64.tar.gz task_windows_amd64.zip; do
    h="$(awk -v f="$f" '$2 == f { print $1 }' "$HERE/go-task.pins")"
    [[ "$h" =~ ^[0-9a-f]{64}$ ]] || PIN_OK=1
done
check "pins a SHA-256 for each of the five runner platforms" "$PIN_OK" "$(cat "$HERE/go-task.pins")"
check "pins a version" "$(awk '$1 == "version" && $2 ~ /^[0-9]+\.[0-9]+\.[0-9]+$/ { f = 1 } END { exit !f }' "$HERE/go-task.pins" && echo 0 || echo 1)"

echo "ci-require-tools.sh"
mkdir -p "$TMP/tools"
printf '#!/usr/bin/env bash\necho "tool 1.2.3"\n' > "$TMP/tools/good"; chmod +x "$TMP/tools/good"
printf '#!/usr/bin/env bash\necho broken >&2; exit 3\n' > "$TMP/tools/broken"; chmod +x "$TMP/tools/broken"
req() { OUT="$(env PATH="$TMP/tools:/usr/bin:/bin" LOCALAPPDATA=/nonexistent bash "$REQUIRE" "$@" 2>&1)"; RC=$?; }

req good
check "a tool that starts passes" "$([ "$RC" -eq 0 ] && echo "$OUT" | grep -q 'OK   good' && echo 0 || echo 1)" "rc=$RC $OUT"
req good missing-one
check "a tool that is not on PATH fails the step" "$([ "$RC" -ne 0 ] && echo "$OUT" | grep -q 'MISSING  missing-one' && echo 0 || echo 1)" "rc=$RC $OUT"
req missing-a good missing-b
check "every missing tool is listed in one run" "$([ "$RC" -ne 0 ] && echo "$OUT" | grep -q 'MISSING  missing-a' && echo "$OUT" | grep -q 'MISSING  missing-b' && echo 0 || echo 1)" "$OUT"
req broken
check "a tool that is on PATH but does not start fails" "$([ "$RC" -ne 0 ] && echo "$OUT" | grep -q "failed" && echo 0 || echo 1)" "$OUT"
req broken=
check "an empty version argument only looks the tool up" "$([ "$RC" -eq 0 ] && echo 0 || echo 1)" "$OUT"
req iscc
check "iscc is reported missing when it is nowhere" "$([ "$RC" -ne 0 ] && echo "$OUT" | grep -q 'MISSING  iscc' && echo 0 || echo 1)" "$OUT"
printf '#!/usr/bin/env bash\nexit 0\n' > "$TMP/tools/iscc"; chmod +x "$TMP/tools/iscc"
req iscc
check "iscc on PATH passes without being run" "$([ "$RC" -eq 0 ] && echo 0 || echo 1)" "$OUT"
req
check "no arguments is a usage error" "$([ "$RC" -eq 2 ] && echo 0 || echo 1)" "rc=$RC"

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]

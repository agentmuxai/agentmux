#!/usr/bin/env bash
# smoke-developer-image.test.sh: tests for scripts/smoke-developer-image.sh, with a fake `docker` that records
# its calls. The script's real work runs inside the image; what is tested here is what runs outside it: the
# arguments, the docker command it builds, the exit code, and that the volumes are removed whatever happens.
#
# Usage: bash scripts/smoke-developer-image.test.sh    (exit 0 = all pass)

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/smoke-developer-image.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0

ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

mkdir -p "$TMP/bin"
cat > "$TMP/bin/docker" <<'DOCKER'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_DOCKER_LOG"
if [ "$1" = run ]; then exit "${FAKE_DOCKER_RUN_EXIT:-0}"; fi
exit 0
DOCKER
chmod +x "$TMP/bin/docker"

run_script() {
    : > "$TMP/docker.log"
    PATH="$TMP/bin:$PATH" FAKE_DOCKER_LOG="$TMP/docker.log" "$@" > "$TMP/out.txt" 2>&1
}

run_script bash "$SCRIPT" --out "$TMP/o1" --image img:1 --ref abc123 --stage dev
rc=$?
[ "$rc" = 0 ] && ok "a passing run exits 0" || bad "a passing run exits 0" "exit $rc"
run_line="$(grep '^run ' "$TMP/docker.log")"
case "$run_line" in *" img:1 bash -c "*) ok "it runs the image it was given" ;; *) bad "it runs the image it was given" "$run_line" ;; esac
case "$run_line" in *"-e REF=abc123"*"-e STAGE=dev"*) ok "the ref and stage reach the container" ;; *) bad "the ref and stage reach the container" "$run_line" ;; esac
case "$run_line" in *":/home/agent/.cargo"*":/home/agent/target"*) ok "cargo's home and the build output are volumes" ;; *) bad "cargo's home and the build output are volumes" "$run_line" ;; esac
[ "$(grep -c '^volume rm' "$TMP/docker.log")" = 1 ] && ok "the volumes are removed after a pass" || bad "the volumes are removed after a pass" "$(cat "$TMP/docker.log")"

FAKE_DOCKER_RUN_EXIT=7 run_script bash "$SCRIPT" --out "$TMP/o2"
rc=$?
[ "$rc" = 7 ] && ok "a failing run passes its exit code on" || bad "a failing run passes its exit code on" "exit $rc"
[ "$(grep -c '^volume rm' "$TMP/docker.log")" = 1 ] && ok "the volumes are removed after a failure" || bad "the volumes are removed after a failure" "$(cat "$TMP/docker.log")"

run_script bash "$SCRIPT" --stage everything
rc=$?
[ "$rc" = 2 ] && ok "an unknown stage is refused" || bad "an unknown stage is refused" "exit $rc"
[ ! -s "$TMP/docker.log" ] && ok "nothing is started for a refused argument" || bad "nothing is started for a refused argument" "$(cat "$TMP/docker.log")"

run_script bash "$SCRIPT" --nope
rc=$?
[ "$rc" = 2 ] && ok "an unknown argument is refused" || bad "an unknown argument is refused" "exit $rc"

run_script bash "$SCRIPT" --help
rc=$?
if [ "$rc" = 0 ] && grep -q -- '--stage' "$TMP/out.txt"; then ok "--help prints the usage and exits 0"; else bad "--help prints the usage and exits 0" "exit $rc"; fi

bash -n "$SCRIPT" && ok "the script parses" || bad "the script parses"

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" = 0 ]

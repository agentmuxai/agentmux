#!/usr/bin/env bash
# Runs inside a distro container that has AgentMux $FROM_VERSION really installed (by apt/dnf/zypper).
#   cases.sh --list        the case names
#   cases.sh <case>        run one case; exit 0 = pass
# The helper is the one the package installed (/opt/agentmux/bin/agentmux-install-update) unless
# HELPER is set. It runs as root directly, against the HTTPS mirror (/www is the mirror's tree).
set -u

FROM_VERSION="${FROM_VERSION:-0.60.3}"
TO_VERSION="${TO_VERSION:-0.60.4}"
HELPER="${HELPER:-/opt/agentmux/bin/agentmux-install-update}"
MIRROR="${MIRROR:-https://mirror.amx-harness.test}"
PKGS=/pkgs
U=$((RANDOM + 100))   # keeps this run's fake releases apart from any other run on the same mirror
UPDATES=/updates

if command -v dpkg-query >/dev/null 2>&1; then KIND=deb; else KIND=rpm; fi

pkg_name() { # version
    if [ "$KIND" = deb ]; then echo "AgentMux_$1_amd64.deb"; else echo "AgentMux-$1-1.x86_64.rpm"; fi
}
installed() {
    if [ "$KIND" = deb ]; then dpkg-query -W -f='${Version}' agentmux 2>/dev/null
    else rpm -q --queryformat '%{VERSION}' agentmux 2>/dev/null; fi
}
install_by_hand() { # file: what a person would type, allowing a downgrade
    if [ "$KIND" = deb ]; then DEBIAN_FRONTEND=noninteractive apt-get install -y --allow-downgrades --no-install-recommends "$1" >/dev/null 2>&1
    elif command -v dnf >/dev/null 2>&1; then dnf -y install --allowerasing "$1" >/dev/null 2>&1 || dnf -y downgrade "$1" >/dev/null 2>&1
    else zypper --non-interactive install --oldpackage --allow-unsigned-rpm "$1" >/dev/null 2>&1; fi
}

# A process that has quit, for the launcher pid: the helper waits for it, so it must be gone.
gone_pid() { sh -c 'exit 0' & local p=$!; wait "$p"; echo "$p"; }

# The mirror's release for <version>, listing <files> the way GitHub lists a release.
fake_release() { # version file...
    local v=$1 f first=1; shift
    mkdir -p "/www/api" "/www/blob/v$v"
    {
        printf '{\n  "tag_name": "v%s",\n  "assets": [\n' "$v"
        for f in "$@"; do
            [ "$first" = 1 ] || printf '    ,\n'
            first=0
            cp "$f" "/www/blob/v$v/$(basename "$f")"
            printf '    {\n      "id": 1,\n      "name": "%s",\n      "size": %s,\n      "digest": "sha256:%s"\n    }\n' \
                "$(basename "$f")" "$(stat -c %s "$f")" "$(sha256sum <"$f" | awk '{print $1}')"
        done
        printf '  ]\n}\n'
    } >"/www/api/v$v"
}
RELEASES=()
scratch_release() { RELEASES+=("$1"); fake_release "$@"; }
cleanup() { for v in "${RELEASES[@]:-}"; do [ -n "$v" ] && rm -rf "/www/api/v$v" "/www/blob/v$v"; done; [ -s /tmp/amx-stub.pid ] && kill "$(cat /tmp/amx-stub.pid)" 2>/dev/null; }
trap cleanup EXIT

# Run the helper as root, the way pkexec would start it. Sets rc, and out/err files.
run_helper() { # args...
    env AGENTMUX_HELPER_TEST=1 AGENTMUX_UPDATE_API="${API:-$MIRROR/api}" AGENTMUX_DOWNLOAD_BASE="$MIRROR/download" \
        AGENTMUX_QUIT_WAIT="${QUIT_WAIT:-6}" "$HELPER" "$@" >/tmp/h.out 2>/tmp/h.err
    rc=$?
}
last_err() { sed 's/^agentmux-install-update: //' /tmp/h.err | tail -n 1; }

fail() { echo "  FAIL: $*"; echo "    rc=$rc out=[$(tr '\n' ' ' </tmp/h.out 2>/dev/null)] err=[$(tail -n 3 /tmp/h.err 2>/dev/null | tr '\n' ' ')] installed=$(installed)"; exit 1; }
pass() { echo "  PASS: $*"; exit 0; }

# The state every case starts from: FROM_VERSION installed, nothing recorded, a user's file in place.
stage() {
    [ "$(installed)" = "$FROM_VERSION" ] || { echo "  SETUP: expected $FROM_VERSION installed, found '$(installed)'"; exit 2; }
    mkdir -p "$UPDATES" && cp "$PKGS/$(pkg_name "$TO_VERSION")" "$UPDATES/"
    chmod 755 "$UPDATES"
    FILE="$UPDATES/$(pkg_name "$TO_VERSION")"
}

# Refused: not zero, no READY (nothing was checked well enough to ask the launcher to quit), the
# installed version unchanged, nothing recorded; the reason matches <regex>.
expect_refused() { # regex description
    [ "$rc" != 0 ] || fail "$2: the helper accepted it"
    ! grep -q '^READY$' /tmp/h.out || fail "$2: READY was printed for a file that should be refused"
    [ "$(installed)" = "${KEPT:-$FROM_VERSION}" ] || fail "$2: the installed version changed to $(installed)"
    [ ! -e /var/lib/agentmux/last-upgrade ] || fail "$2: an upgrade was recorded"
    grep -Eq "$1" /tmp/h.err || fail "$2: reason doesn't match /$1/"
    pass "$2 ($(last_err))"
}

list() { grep -o '^case_[a-z_0-9]*' "$0" | sed 's/^case_//'; }

case_wrong_file_name() {
    stage
    mv "$FILE" "$UPDATES/evil.$KIND"
    run_helper "$UPDATES/evil.$KIND" "$TO_VERSION" "$(gone_pid)"
    expect_refused "isn't the AgentMux $TO_VERSION package" "a file not named for the version"
}

case_symlink() {
    stage
    mv "$FILE" "$UPDATES/real.$KIND"
    ln -sf "$UPDATES/real.$KIND" "$FILE"
    run_helper "$FILE" "$TO_VERSION" "$(gone_pid)"
    expect_refused "is a symlink" "a symlink in place of the file"
}

case_relative_path() {
    stage
    run_helper "AgentMux_${TO_VERSION}_amd64.$KIND" "$TO_VERSION" "$(gone_pid)"
    expect_refused "absolute path" "a relative path"
}

case_real_github_upgrade_and_go_back() {
    # No mirror: the listing and the download come from github.com itself.
    stage
    env -u AGENTMUX_UPDATE_API -u AGENTMUX_DOWNLOAD_BASE AGENTMUX_QUIT_WAIT=6 "$HELPER" "$FILE" "$TO_VERSION" "$(gone_pid)" >/tmp/h.out 2>/tmp/h.err; rc=$?
    [ "$rc" = 0 ] && [ "$(installed)" = "$TO_VERSION" ] || fail "upgrade against github.com failed"
    env -u AGENTMUX_UPDATE_API -u AGENTMUX_DOWNLOAD_BASE AGENTMUX_QUIT_WAIT=6 "$HELPER" --go-back "$FROM_VERSION" "$(gone_pid)" >/tmp/h.out 2>/tmp/h.err; rc=$?
    [ "$rc" = 0 ] && [ "$(installed)" = "$FROM_VERSION" ] || fail "go-back from github.com failed"
    pass "real github.com: $FROM_VERSION -> $TO_VERSION (GitHub's own checksum), then back to $FROM_VERSION (downloaded from the release)"
}

# #5717: the 0.60.5 helper and launcher in a real 0.60.4 install do the upgrade (the real signed
# manifest, with GitHub's API unreachable so a fall back would fail), then go back the same way.
case_real_upgrade_through_the_new_helper() {
    stage
    mkdir -p /tmp/newpkg && cd /tmp/newpkg || fail "no scratch"
    if [ "$KIND" = deb ]; then dpkg-deb -x "$FILE" . ; else command -v cpio >/dev/null || dnf -y install cpio >/dev/null 2>&1; rpm2cpio "$FILE" | cpio -idm 2>/dev/null; fi
    [ -x opt/agentmux/bin/agentmux-install-update ] && [ -x opt/agentmux/bin/agentmux-launcher ] || fail "the $TO_VERSION package lacks the helper or launcher"
    # What the package installs for the helper pair, put over the old install.
    cp opt/agentmux/bin/agentmux-launcher opt/agentmux/bin/agentmux-install-update /opt/agentmux/bin/ || fail "can't swap in the new pair"
    cd / || exit 1
    env -u AGENTMUX_DOWNLOAD_BASE AGENTMUX_UPDATE_API=https://invalid.test/api AGENTMUX_QUIT_WAIT=6 "$HELPER" "$FILE" "$TO_VERSION" "$(gone_pid)" >/tmp/h.out 2>/tmp/h.err; rc=$?
    [ "$rc" = 0 ] && grep -q '^READY$' /tmp/h.out && [ "$(installed)" = "$TO_VERSION" ] || fail "upgrade through the new helper failed: rc=$rc $(cat /tmp/h.out /tmp/h.err)"
    [ "$(grep -c '^to=' /var/lib/agentmux/last-upgrade)" = 1 ] && grep -q "^to=$TO_VERSION" /var/lib/agentmux/last-upgrade || fail "no record: $(cat /var/lib/agentmux/last-upgrade 2>&1)"
    env -u AGENTMUX_DOWNLOAD_BASE AGENTMUX_UPDATE_API=https://invalid.test/api AGENTMUX_QUIT_WAIT=6 "$HELPER" --go-back "$FROM_VERSION" "$(gone_pid)" >/tmp/h.out 2>/tmp/h.err; rc=$?
    [ "$rc" = 0 ] && [ "$(installed)" = "$FROM_VERSION" ] || fail "go-back through the new helper failed: rc=$rc $(cat /tmp/h.out /tmp/h.err)"
    pass "real $FROM_VERSION -> $TO_VERSION through the new helper+launcher (signed manifest, API unreachable), then back to $FROM_VERSION the same way"
}

case_refuses_a_corrupted_real_file() {
    stage
    cp "$FILE" /tmp/bad && printf x >> /tmp/bad && cp /tmp/bad "$FILE"
    mkdir -p /tmp/newpkg && cd /tmp/newpkg || fail "no scratch"
    good="$PKGS/$(pkg_name "$TO_VERSION")"
    if [ "$KIND" = deb ]; then dpkg-deb -x "$good" . ; else command -v cpio >/dev/null || dnf -y install cpio >/dev/null 2>&1; rpm2cpio "$good" | cpio -idm 2>/dev/null; fi
    cp opt/agentmux/bin/agentmux-launcher opt/agentmux/bin/agentmux-install-update /opt/agentmux/bin/ || fail "can't swap in the new pair"
    cd / || exit 1
    env -u AGENTMUX_DOWNLOAD_BASE AGENTMUX_UPDATE_API=https://invalid.test/api AGENTMUX_QUIT_WAIT=6 "$HELPER" "$FILE" "$TO_VERSION" "$(gone_pid)" >/tmp/h.out 2>/tmp/h.err; rc=$?
    [ "$rc" != 0 ] && ! grep -q READY /tmp/h.out && grep -q "signature on v$TO_VERSION doesn't match" /tmp/h.err && [ "$(installed)" = "$FROM_VERSION" ] || fail "not refused: rc=$rc $(cat /tmp/h.out /tmp/h.err)"
    pass "a corrupted $TO_VERSION package is refused with the signature message, nothing installed"
}

# The launcher's own waiter (`WAITER` in crates/launcher/src/upgrade_package.rs, mounted at /upgrade_package.rs
# by run.sh) around the real helper: pkexec is a stand-in that runs the helper directly (this is root already),
# and the start command records the version installed at the moment it ran. Needs FROM_VERSION >= 0.60.5.
waiter_setup() {
    [ -f /upgrade_package.rs ] || { echo "  SETUP: the launcher source isn't mounted (run.sh mounts crates/launcher/src/upgrade_package.rs)"; exit 2; }
    awk '/^const WAITER: &str = r#"/{sub(/^const WAITER: &str = r#"/,""); f=1} f{ if ($0 ~ /^"#;/) exit; print }' /upgrade_package.rs >/tmp/waiter.sh
    [ -s /tmp/waiter.sh ] || { echo "  SETUP: couldn't read WAITER from the launcher source"; exit 2; }
    printf '#!/bin/sh\nexec "$@"\n' >/tmp/pkexec; chmod 755 /tmp/pkexec
    cat >/tmp/start <<'STUB'
#!/bin/sh
if command -v dpkg-query >/dev/null 2>&1; then v=$(dpkg-query -W -f='${Version}' agentmux); else v=$(rpm -q --queryformat '%{VERSION}' agentmux); fi
if kill -0 "$(cat /tmp/lpid)" 2>/dev/null; then g=launcher-still-running; else g=launcher-gone; fi
echo "$v $g" >/tmp/started
STUB
    chmod 755 /tmp/start
    rm -f /tmp/started /tmp/w.out /tmp/w.err /tmp/w.result
    # The launcher: alive while the helper checks the file, gone a little after READY.
    sleep "${LAUNCHER_LIFE:-5}" & echo $! >/tmp/lpid
}
run_waiter() { # file
    env -u AGENTMUX_UPDATE_API -u AGENTMUX_DOWNLOAD_BASE AGENTMUX_QUIT_WAIT=20 sh -c "$(cat /tmp/waiter.sh)" agentmux-install-waiter \
        /tmp/pkexec "$HELPER" "$1" "$TO_VERSION" "$(cat /tmp/lpid)" /tmp/w.out /tmp/w.err /tmp/w.result /tmp/start
}

case_waiter_installs_then_starts_the_new_version() {
    stage; waiter_setup
    run_waiter "$FILE"; rc=$?
    cp /tmp/w.out /tmp/h.out; cp /tmp/w.err /tmp/h.err
    [ "$rc" = 0 ] && grep -qx READY /tmp/w.out || fail "the waiter didn't finish a clean install"
    [ "$(installed)" = "$TO_VERSION" ] || fail "installed is $(installed)"
    [ ! -e /tmp/w.result ] || fail "a result was left for a clean install: $(cat /tmp/w.result)"
    [ "$(cat /tmp/started 2>/dev/null)" = "$TO_VERSION launcher-gone" ] || fail "start ran as [$(cat /tmp/started 2>&1)], not as the new version after the launcher left"
    pass "READY, installed $TO_VERSION, no result left, and AgentMux was started as $TO_VERSION only after the launcher was gone"
}

case_waiter_leaves_the_old_version_running_when_the_file_is_refused() {
    stage; waiter_setup
    cp "$FILE" /tmp/bad && printf x >> /tmp/bad && cp /tmp/bad "$FILE"
    run_waiter "$FILE"; rc=$?
    cp /tmp/w.out /tmp/h.out; cp /tmp/w.err /tmp/h.err
    [ "$rc" != 0 ] && ! grep -qx READY /tmp/w.out || fail "a corrupt file got READY"
    [ "$(installed)" = "$FROM_VERSION" ] || fail "installed is $(installed)"
    [ ! -e /tmp/started ] || fail "the waiter started AgentMux although the launcher was never told to quit"
    grep -q "doesn't match" /tmp/w.err || fail "no signature reason: $(cat /tmp/w.err)"
    pass "a corrupt file never reached READY: nothing installed, the launcher kept running, no second AgentMux started"
}

case_waiter_restarts_the_old_version_when_the_install_fails_after_ready() {
    stage; waiter_setup
    run_waiter "$FILE" & local w=$!
    for _ in $(seq 1 60); do grep -qx READY /tmp/w.out 2>/dev/null && break; sleep 0.5; done
    grep -qx READY /tmp/w.out || fail "no READY"
    # The launcher is still running; the package manager breaks before the helper reaches it.
    for pm in apt-get dnf zypper; do
        t=$(command -v "$pm" 2>/dev/null) && { mv "$t" "$t.real"; printf '#!/bin/sh\necho "simulated failure" >&2\nexit 100\n' >"$t"; chmod 755 "$t"; }
    done
    wait "$w"; rc=$?
    for pm in apt-get dnf zypper; do t=$(command -v "$pm" 2>/dev/null); [ -n "$t" ] && [ -e "$t.real" ] && mv -f "$t.real" "$t"; done
    cp /tmp/w.out /tmp/h.out; cp /tmp/w.err /tmp/h.err
    [ "$(installed)" = "$FROM_VERSION" ] || fail "installed is $(installed)"
    grep -q '"error"' /tmp/w.result 2>/dev/null || fail "no reason left for srv: $(cat /tmp/w.result 2>&1)"
    [ "$(cat /tmp/started 2>/dev/null)" = "$FROM_VERSION launcher-gone" ] || fail "start ran as [$(cat /tmp/started 2>&1)], not the old version after the launcher left"
    pass "the install failed after READY: the reason was left for srv ($(tr -d '\n' </tmp/w.result | cut -c1-90)) and $FROM_VERSION was started again"
}

case_must_run_as_root() {
    stage
    id other >/dev/null 2>&1 || useradd -m other
    setpriv --reuid=other --regid=other --init-groups "$HELPER" "$FILE" "$TO_VERSION" "$(gone_pid)" >/tmp/h.out 2>/tmp/h.err; rc=$?
    expect_refused "must run as root" "the helper started without root"
}

case_pkexec_fails_without_an_agent() {
    stage
    command -v pkexec >/dev/null || { echo "  SETUP: no pkexec in this image"; exit 2; }
    id other >/dev/null 2>&1 || useradd -m other
    # What the launcher runs, as the desktop user, with no polkit agent or daemon in a container.
    setpriv --reuid=other --regid=other --init-groups timeout 30 pkexec "$HELPER" "$FILE" "$TO_VERSION" "$(gone_pid)" >/tmp/h.out 2>/tmp/h.err </dev/null; rc=$?
    { [ "$rc" = 126 ] || [ "$rc" = 127 ]; } || fail "pkexec exit status $rc is not one the launcher turns into 'install it yourself' (126/127)"
    ! grep -q '^READY$' /tmp/h.out || fail "READY with no authorisation"
    [ "$(installed)" = "$FROM_VERSION" ] || fail "something was installed"
    # The command the panel prints (UpgradePackage::manual_command), run as root.
    if [ "$KIND" = deb ]; then manual="apt install -y $FILE"; elif command -v dnf >/dev/null 2>&1; then manual="dnf install -y $FILE"; else manual="zypper --non-interactive install --allow-unsigned-rpm $FILE"; fi
    DEBIAN_FRONTEND=noninteractive $manual >/tmp/m.out 2>&1 || { cat /tmp/m.out | tail -5; fail "the printed command '$manual' failed"; }
    [ "$(installed)" = "$TO_VERSION" ] || fail "the printed command left $(installed) installed"
    pass "pkexec refused with status $rc (no READY, nothing installed); the printed command '$manual' installed $TO_VERSION"
}

if [ "${1:-}" = --list ]; then list; exit 0; fi
[ -n "${1:-}" ] && declare -F "case_$1" >/dev/null || { echo "usage: cases.sh --list | <case>"; exit 64; }
"case_$1"

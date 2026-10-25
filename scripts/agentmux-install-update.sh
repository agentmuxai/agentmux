#!/bin/sh
# agentmux-install-update <file> <version> <launcher pid>
#
# Installs an AgentMux .deb or .rpm that AgentMux downloaded for its own upgrade.
# Shipped by both packages as /opt/agentmux/bin/agentmux-install-update and run as
# root through pkexec (polkit action ai.agentmux.install-update), so the desktop
# asks for the user's password. See docs/specs/SPEC_LINUX_PACKAGE_UPGRADE_AND_REPOSITORY_2026_10_24.md §3.2.
#
# It trusts nothing it is given except as a request. Every argument comes from a
# process running as the user:
#   - the file is copied (not followed through a symlink) into a root-only
#     directory and only the copy is used, so it can't change after it is checked;
#   - the checksum comes from GitHub's own release listing, fetched here, never from
#     the caller, so naming a hash can't get a file of one's own installed as root;
#   - the package must be `agentmux`, exactly <version>, for this architecture, and
#     newer than the installed one;
#   - nothing may run from /opt/agentmux (any user's AgentMux) once the launcher has
#     quit, since replacing libcef.so under a running CEF can crash it.
#
# On stdout it prints READY once everything is checked and it is waiting for the
# launcher to quit; the launcher reads that to know the password was accepted and
# the file is good. Failures are one line on stderr and a non-zero status.
set -u
umask 077
PATH=/usr/sbin:/usr/bin:/sbin:/bin
# A test run directly puts its fake package tools first; under pkexec neither
# variable exists.
[ -n "${AGENTMUX_HELPER_TEST:-}" ] && PATH="${AGENTMUX_HELPER_PATH:-}:$PATH"
export PATH
LC_ALL=C
export LC_ALL

REPO_API="${AGENTMUX_UPDATE_API:-https://api.github.com/repos/agentmuxai/agentmux/releases/tags}"
INSTALL_ROOT="${AGENTMUX_INSTALL_ROOT:-/opt/agentmux}"
CACHE_DIR="${AGENTMUX_CACHE_DIR:-/var/cache/agentmux}"
STATE_DIR="${AGENTMUX_STATE_DIR:-/var/lib/agentmux}"
PROC_DIR="${AGENTMUX_PROC_DIR:-/proc}"
QUIT_WAIT="${AGENTMUX_QUIT_WAIT:-120}"

die() {
    echo "agentmux-install-update: $*" >&2
    exit 1
}

# Under pkexec the environment is reset, so the variables above are only ever set
# by a test run directly.
if [ -z "${AGENTMUX_HELPER_TEST:-}" ] && [ "$(id -u)" != 0 ]; then
    die "must run as root (through pkexec)"
fi
[ "$#" -eq 3 ] || die "usage: agentmux-install-update <file> <version> <launcher pid>"
FILE=$1
VERSION=$2
LAUNCHER_PID=$3

case "$VERSION" in
'' | *[!0-9A-Za-z.+~-]*) die "not a version: $VERSION" ;;
esac
case "$LAUNCHER_PID" in
'' | *[!0-9]*) die "not a process id: $LAUNCHER_PID" ;;
esac
case "$FILE" in
/*) ;;
*) die "the file must be an absolute path" ;;
esac

# --- the file: a regular file of the invoking user, copied into a directory only
# root can enter, without following a symlink ---
[ ! -L "$FILE" ] || die "$FILE is a symlink"
[ -f "$FILE" ] || die "$FILE isn't a file"
if [ -n "${PKEXEC_UID:-}" ]; then
    [ "$(stat -c %u -- "$FILE" 2>/dev/null)" = "$PKEXEC_UID" ] || die "$FILE isn't the invoking user's file"
fi
NAME=$(basename -- "$FILE")
case "$NAME" in
"AgentMux_${VERSION}_amd64.deb") KIND=deb ;;
"AgentMux-${VERSION}-1.x86_64.rpm") KIND=rpm ;;
*) die "$NAME isn't the AgentMux $VERSION package" ;;
esac

mkdir -p "$CACHE_DIR" || die "can't create $CACHE_DIR"
chmod 700 "$CACHE_DIR" || die "can't secure $CACHE_DIR"
WORK=$(mktemp -d "$CACHE_DIR/install.XXXXXX") || die "can't make a work folder"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT
COPY="$WORK/$NAME"
dd if="$FILE" of="$COPY" iflag=nofollow bs=1M status=none 2>/dev/null || die "can't read $FILE"
chmod 600 "$COPY"

# --- the checksum, from GitHub itself ---
fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsS --proto '=https' --tlsv1.2 --max-time 30 "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O - --secure-protocol=TLSv1_2 --timeout=30 "$1"
    else
        return 127
    fi
}
LISTING=$(fetch "$REPO_API/v$VERSION") || die "can't read GitHub's listing of AgentMux $VERSION to check the file"
# The release listing is pretty-printed JSON: an asset's "name" line is followed by
# its "digest" line ("sha256:<hex>").
EXPECTED=$(printf '%s\n' "$LISTING" | awk -v n="\"name\": \"$NAME\"" '
    index($0, n) { found = 1; next }
    found && /"digest"/ { sub(/.*"sha256:/, ""); sub(/".*/, ""); print; exit }
    found && /"name"/ { exit }')
case "$EXPECTED" in
'' | *[!0-9a-f]*) die "GitHub lists no checksum for $NAME" ;;
esac
[ "${#EXPECTED}" -eq 64 ] || die "GitHub lists no checksum for $NAME"
ACTUAL=$(sha256sum <"$COPY" | awk '{print $1}')
[ "$ACTUAL" = "$EXPECTED" ] || die "$NAME doesn't match the checksum GitHub lists for it"

# --- which package manager owns this install ---
LAUNCHER="$INSTALL_ROOT/bin/agentmux-launcher"
if dpkg-query -S "$LAUNCHER" >/dev/null 2>&1; then
    OWNER=deb
elif rpm -qf "$LAUNCHER" >/dev/null 2>&1; then
    OWNER=rpm
else
    die "AgentMux isn't installed by dpkg or rpm"
fi
[ "$OWNER" = "$KIND" ] || die "AgentMux was installed from a .$OWNER, not a .$KIND"

# --- the package itself ---
ARCH_NOW=$(uname -m)
if [ "$KIND" = deb ]; then
    PKG_NAME=$(dpkg-deb --field "$COPY" Package 2>/dev/null) || die "$NAME isn't a .deb"
    PKG_VERSION=$(dpkg-deb --field "$COPY" Version 2>/dev/null)
    PKG_ARCH=$(dpkg-deb --field "$COPY" Architecture 2>/dev/null)
    [ "$ARCH_NOW" = x86_64 ] && [ "$PKG_ARCH" = amd64 ] || die "$NAME is for $PKG_ARCH, not this computer"
    INSTALLED=$(dpkg-query -W -f='${Version}' agentmux 2>/dev/null)
else
    PKG_NAME=$(rpm -qp --queryformat '%{NAME}' "$COPY" 2>/dev/null) || die "$NAME isn't an .rpm"
    PKG_VERSION=$(rpm -qp --queryformat '%{VERSION}' "$COPY" 2>/dev/null)
    PKG_ARCH=$(rpm -qp --queryformat '%{ARCH}' "$COPY" 2>/dev/null)
    [ "$ARCH_NOW" = x86_64 ] && [ "$PKG_ARCH" = x86_64 ] || die "$NAME is for $PKG_ARCH, not this computer"
    INSTALLED=$(rpm -q --queryformat '%{VERSION}' agentmux 2>/dev/null)
fi
[ "$PKG_NAME" = agentmux ] || die "$NAME is the package '$PKG_NAME', not agentmux"
[ "$PKG_VERSION" = "$VERSION" ] || die "$NAME is version $PKG_VERSION, not $VERSION"
[ -n "$INSTALLED" ] || die "can't tell which AgentMux version is installed"
newer() {
    if [ "$KIND" = deb ]; then
        dpkg --compare-versions "$1" gt "$2"
    else
        [ "$(rpm --eval "%{lua: print(rpm.vercmp('$1', '$2'))}" 2>/dev/null)" = 1 ]
    fi
}
newer "$VERSION" "$INSTALLED" || die "$VERSION isn't newer than the installed $INSTALLED"

# --- everything checked: tell the launcher, then wait for AgentMux to quit ---
echo READY
waited=0
while kill -0 "$LAUNCHER_PID" 2>/dev/null; do
    [ "$waited" -lt "$QUIT_WAIT" ] || die "AgentMux didn't quit"
    sleep 1
    waited=$((waited + 1))
done

# Nothing of any user may run from the install: its helper processes can outlive
# the launcher briefly, so give them the rest of the wait.
running_from_install() {
    for p in "$PROC_DIR"/[0-9]*; do
        exe=$(readlink "$p/exe" 2>/dev/null) || continue
        case "$exe" in
        "$INSTALL_ROOT"/*) return 0 ;;
        esac
    done
    return 1
}
while running_from_install; do
    [ "$waited" -lt "$QUIT_WAIT" ] || die "AgentMux is still running (for another user?), so it can't be replaced"
    sleep 1
    waited=$((waited + 1))
done

# --- the install ---
if [ "$KIND" = deb ]; then
    if command -v apt-get >/dev/null 2>&1; then
        DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends -o Dpkg::Options::=--force-confold "$COPY" >&2 \
            || die "apt-get couldn't install $NAME"
    else
        dpkg --install "$COPY" >&2 || die "dpkg couldn't install $NAME"
    fi
else
    if command -v dnf >/dev/null 2>&1; then
        dnf install -y "$COPY" >&2 || die "dnf couldn't install $NAME"
    elif command -v zypper >/dev/null 2>&1; then
        zypper --non-interactive install --allow-unsigned-rpm "$COPY" >&2 || die "zypper couldn't install $NAME"
    else
        rpm -U "$COPY" >&2 || die "rpm couldn't install $NAME"
    fi
fi

# Its record, for a later "go back": what it replaced and what it installed.
mkdir -p "$STATE_DIR" && printf 'from=%s\nto=%s\n' "$INSTALLED" "$VERSION" >"$STATE_DIR/last-upgrade" 2>/dev/null
exit 0

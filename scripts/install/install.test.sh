#!/bin/bash
# Tests for scripts/install/install.sh: dry runs for each platform, the refusals, and the download checks
# against a signed manifest made here with a throwaway Ed25519 key. Needs OpenSSL 3 and node (for the key-sync check).
# Run:  bash scripts/install/install.test.sh   (from anywhere)
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/install.sh"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
W="$(mktemp -d)"
trap 'rm -rf "$W"' EXIT
FAILS=0

pass() { echo "ok   - $1"; }
# Portable: GNU sed -i and sha256sum are not on a stock Mac (BSD sed reads the next word as a backup suffix).
sha256_hex() { # the file on stdin
    if command -v sha256sum > /dev/null 2>&1; then
        sha256sum | cut -d ' ' -f 1
    else
        shasum -a 256 | cut -d ' ' -f 1
    fi
}
sed_inplace() { # <sed expression> <file>
    sed "$1" "$2" > "$2.tmp" && mv "$2.tmp" "$2"
}
fail() { echo "FAIL - $1"; FAILS=$((FAILS + 1)); }
# expect <name> <expected exit> <must contain, or empty> <command...>
expect() {
    name="$1"; code="$2"; want="$3"; shift 3
    out="$("$@" 2>&1)"; got=$?
    if [ "$got" != "$code" ]; then fail "$name (exit $got, wanted $code): $out"; return; fi
    if [ -n "$want" ] && ! printf '%s' "$out" | grep -qF -- "$want"; then fail "$name (missing '$want'): $out"; return; fi
    pass "$name"
}
refuses_to_contain() { # <name> <text> <command...>
    name="$1"; bad="$2"; shift 2
    out="$("$@" 2>&1)"
    if printf '%s' "$out" | grep -qF -- "$bad"; then fail "$name (contains '$bad'): $out"; else pass "$name"; fi
}

for SH in sh bash dash; do
    command -v "$SH" > /dev/null 2>&1 || continue
    echo "== $SH =="
    "$SH" -n "$SCRIPT" && pass "$SH parses the script" || fail "$SH parses the script"

    expect "--help" 0 "Usage: install.sh" "$SH" "$SCRIPT" --help
    expect "unknown option" 1 "unknown option" "$SH" "$SCRIPT" --nope
    expect "bad --version" 1 "--version must look like" "$SH" "$SCRIPT" --version "1;rm -rf /"
    expect "bad --method" 1 "--method is" "$SH" "$SCRIPT" --method magic

    export AGENTMUX_INSTALL_ROOT="$W/empty-root"
    mkdir -p "$AGENTMUX_INSTALL_ROOT"
    export HOME="$W/home"
    mkdir -p "$HOME"

    # Linux, by package manager
    export AGENTMUX_INSTALL_OS=Linux AGENTMUX_INSTALL_ARCH=x86_64
    AGENTMUX_INSTALL_PM=apt expect "linux apt dry run" 0 "+ fetch https://packages.agentmux.ai/agentmux.sources -> /etc/apt/sources.list.d/agentmux.sources" "$SH" "$SCRIPT" --dry-run --no-launch
    AGENTMUX_INSTALL_PM=apt expect "linux apt installs the package" 0 "apt-get install -y agentmux" "$SH" "$SCRIPT" --dry-run --no-launch
    AGENTMUX_INSTALL_PM=apt expect "linux apt keyring" 0 "/usr/share/keyrings/agentmux-archive-keyring.gpg" "$SH" "$SCRIPT" --dry-run --no-launch
    AGENTMUX_INSTALL_PM=dnf expect "linux dnf dry run" 0 "dnf install -y agentmux" "$SH" "$SCRIPT" --dry-run --no-launch
    AGENTMUX_INSTALL_PM=dnf expect "linux dnf repo file" 0 "-> /etc/yum.repos.d/agentmux.repo" "$SH" "$SCRIPT" --dry-run --no-launch
    AGENTMUX_INSTALL_PM=zypper expect "linux zypper dry run" 0 "zypper --non-interactive install -y agentmux" "$SH" "$SCRIPT" --dry-run --no-launch
    AGENTMUX_INSTALL_PM=apt expect "package-manager method names a version note" 0 "--version is not used" "$SH" "$SCRIPT" --dry-run --no-launch --version 0.60.14
    AGENTMUX_INSTALL_PM=apt expect "package-manager mode ends with done" 0 "done" "$SH" "$SCRIPT" --dry-run --no-launch

    # Linux, no package manager: the portable download (a version is given, so no network is needed for a dry run)
    AGENTMUX_INSTALL_PM=none expect "linux portable dry run" 0 "AgentMux_0.60.14_amd64-portable.tar.xz" "$SH" "$SCRIPT" --dry-run --no-launch --version 0.60.14 --method download
    AGENTMUX_INSTALL_PM=apt expect "--method package-manager with none" 1 "no supported package manager" env AGENTMUX_INSTALL_PM=none "$SH" "$SCRIPT" --dry-run --method package-manager

    # Platforms with no build
    AGENTMUX_INSTALL_ARCH=aarch64 expect "linux arm64 has no build" 1 "no arm64 Linux build" "$SH" "$SCRIPT" --dry-run
    AGENTMUX_INSTALL_OS=Darwin AGENTMUX_INSTALL_ARCH=x86_64 expect "intel mac has no build" 1 "no Intel Mac build" "$SH" "$SCRIPT" --dry-run
    AGENTMUX_INSTALL_OS=MINGW64_NT expect "windows points at install.ps1" 1 "install.ps1" "$SH" "$SCRIPT" --dry-run
    AGENTMUX_INSTALL_OS=FreeBSD expect "unknown system" 1 "unsupported system" "$SH" "$SCRIPT" --dry-run

    # macOS, with no brew cask: the dmg
    AGENTMUX_INSTALL_OS=Darwin AGENTMUX_INSTALL_ARCH=arm64 expect "macos dmg dry run" 0 "AgentMux_0.60.14_arm64.dmg" "$SH" "$SCRIPT" --dry-run --no-launch --version 0.60.14 --method download

    # Already installed: nothing changes, exit 0
    mkdir -p "$W/root-linux/opt/agentmux"
    AGENTMUX_INSTALL_ROOT="$W/root-linux" AGENTMUX_INSTALL_PM=apt expect "linux already installed" 0 "already installed" "$SH" "$SCRIPT" --no-launch
    AGENTMUX_INSTALL_ROOT="$W/root-linux" AGENTMUX_INSTALL_PM=apt refuses_to_contain "linux already installed runs nothing" "+ " "$SH" "$SCRIPT" --no-launch
    AGENTMUX_INSTALL_ROOT="$W/root-linux" AGENTMUX_INSTALL_PM=apt expect "--force installs over it" 0 "apt-get install -y agentmux" "$SH" "$SCRIPT" --dry-run --no-launch --force
    mkdir -p "$W/root-mac/Applications/AgentMux.app"
    AGENTMUX_INSTALL_ROOT="$W/root-mac" AGENTMUX_INSTALL_OS=Darwin AGENTMUX_INSTALL_ARCH=arm64 expect "macos already installed" 0 "/Applications/AgentMux.app" "$SH" "$SCRIPT" --no-launch

    # The server package: all four platforms, from the signed release, and what the desktop install does not touch
    AGENTMUX_INSTALL_PM=apt expect "server dry run, linux x64" 0 "agentmux-headless-0.60.14-linux-x64.tar.gz" "$SH" "$SCRIPT" --headless --dry-run --version 0.60.14
    AGENTMUX_INSTALL_ARCH=aarch64 expect "server dry run, linux arm64" 0 "agentmux-headless-0.60.14-linux-arm64.tar.gz" "$SH" "$SCRIPT" --headless --dry-run --version 0.60.14
    AGENTMUX_INSTALL_OS=Darwin AGENTMUX_INSTALL_ARCH=arm64 expect "server dry run, macos arm64" 0 "agentmux-headless-0.60.14-macos-arm64.tar.gz" "$SH" "$SCRIPT" --headless --dry-run --version 0.60.14
    AGENTMUX_INSTALL_OS=Darwin AGENTMUX_INSTALL_ARCH=x86_64 expect "the server has no Intel Mac package: refused up front" 1 "no Intel Mac Headless package" "$SH" "$SCRIPT" --headless --dry-run --version 0.60.14
    AGENTMUX_INSTALL_OS=Darwin AGENTMUX_INSTALL_ARCH=x86_64 refuses_to_contain "the Intel Mac refusal comes before any download or plan" "+ " "$SH" "$SCRIPT" --headless --dry-run --version 0.60.14
    AGENTMUX_INSTALL_OS=Darwin AGENTMUX_INSTALL_ARCH=x86_64 expect "the Intel Mac refusal needs no network (no version given)" 1 "no Intel Mac Headless package" "$SH" "$SCRIPT" --headless --dry-run
    expect "server dry run links the program under both names" 0 "/.local/bin/agentmux-headless" "$SH" "$SCRIPT" --headless --dry-run --version 0.60.14
    expect "--server still works, under the new package name" 0 "agentmux-headless-0.60.14-linux-x64.tar.gz" "$SH" "$SCRIPT" --server --dry-run --version 0.60.14
    expect "server dry run with --setup runs setup" 0 "agentmux-headless setup --yes --no-open" "$SH" "$SCRIPT" --headless --setup --dry-run --version 0.60.14
    AGENTMUX_INSTALL_ROOT="$W/root-linux" AGENTMUX_INSTALL_PM=apt expect "an installed desktop app does not stop the server install" 0 "agentmux-headless-0.60.14-linux-x64.tar.gz" "$SH" "$SCRIPT" --headless --dry-run --version 0.60.14
    expect "the server has no package-manager method" 1 "no package-manager method" "$SH" "$SCRIPT" --headless --method package-manager --dry-run
    expect "--setup is for --headless" 1 "--setup is for --headless" "$SH" "$SCRIPT" --setup --dry-run
    AGENTMUX_INSTALL_OS=FreeBSD expect "the server has no build for an unknown system" 1 "unsupported system" "$SH" "$SCRIPT" --headless --dry-run --version 0.60.14

    # ASCII only, no smart quotes
    if LC_ALL=C grep -nP '[^\x00-\x7F]' "$SCRIPT" > /dev/null; then fail "install.sh is ASCII only"; else pass "install.sh is ASCII only"; fi
    unset AGENTMUX_INSTALL_OS AGENTMUX_INSTALL_ARCH
done

# ---- the pinned keys are the app's keys ----
if command -v node > /dev/null 2>&1; then
    want="$(node -e 'const k=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8")).keys;console.log(k.map(x=>x.id+":"+x.public_key).join(" "))' "$REPO_ROOT/crates/common/update-keys.json")"
    have="$(sed -n 's/^UPDATE_KEYS="\(.*\)"$/\1/p' "$SCRIPT")"
    if [ "$want" = "$have" ]; then pass "UPDATE_KEYS equals crates/common/update-keys.json"; else fail "UPDATE_KEYS ('$have') differs from crates/common/update-keys.json ('$want')"; fi
else
    echo "skip - node not found, key sync not checked"
fi
if command -v node > /dev/null 2>&1; then
    want="$(sed -n 's/^export const SSHSIG_FLOOR = "\(.*\)";$/\1/p' "$REPO_ROOT/scripts/release-manifest.mjs")"
    have="$(sed -n 's/^SSHSIG_FLOOR="\(.*\)"$/\1/p' "$SCRIPT")"
    if [ -n "$want" ] && [ "$want" = "$have" ]; then pass "SSHSIG_FLOOR equals scripts/release-manifest.mjs's"; else fail "SSHSIG_FLOOR ('$have') differs from scripts/release-manifest.mjs ('$want')"; fi
fi

# ---- the download checks, against a release made here ----
if ! openssl pkeyutl -help 2>&1 | grep -q -- '-rawin'; then
    echo "skip - this openssl cannot check Ed25519 signatures; download checks not run"
else
    echo "== download checks =="
    VER=9.9.9
    NAME="AgentMux_${VER}_amd64-portable.tar.xz"
    openssl genpkey -algorithm ed25519 -out "$W/priv.pem" 2> /dev/null
    openssl pkey -in "$W/priv.pem" -pubout -outform DER 2> /dev/null | tail -c 32 | openssl base64 -A > "$W/pub.b64"
    TESTKEYS="t1:$(cat "$W/pub.b64")"

    # make_release <dir> <tag> [name to list in the manifest] : a tar.xz with an AgentMux/agentmux.sh, its manifest, the envelope and the signature
    make_release() {
        d="$1"; tag="$2"; listed="${3:-$NAME}"
        mkdir -p "$d" "$W/pkg/AgentMux"
        printf '#!/bin/sh\necho hello\n' > "$W/pkg/AgentMux/agentmux.sh"; chmod +x "$W/pkg/AgentMux/agentmux.sh"
        tar -cJf "$d/$NAME" -C "$W/pkg" AgentMux
        sign_release "$d" "$tag" "$listed" "$NAME"
    }
    # sign_release <dir> <tag> <name to list in the manifest> <the file in the dir it describes>
    sign_release() {
        d="$1"; tag="$2"; listed="$3"; file="$4"
        size="$(wc -c < "$d/$file" | tr -d ' ')"
        sha="$(sha256_hex < "$d/$file")"
        # The same shape release-manifest.mjs writes: indented JSON, each file as name, size, sha256.
        printf '{\n  "format": 1,\n  "version": "%s",\n  "tag": "%s",\n  "files": [\n    {\n      "name": "%s",\n      "size": %s,\n      "sha256": "%s"\n    }\n  ]\n}\n' "$VER" "$tag" "$listed" "$size" "$sha" > "$d/AgentMux-$VER-manifest.json"
        msha="$(sha256_hex < "$d/AgentMux-$VER-manifest.json")"
        printf '{\n  "format": 1,\n  "tag": "%s",\n  "manifest_sha256": "%s",\n  "created_at": "2026-11-04T00:00:00Z"\n}\n' "$tag" "$msha" > "$d/AgentMux-$VER-envelope.json"
        printf 't1 %s\n' "$(openssl pkeyutl -sign -inkey "$W/priv.pem" -rawin -in "$d/AgentMux-$VER-envelope.json" | openssl base64 -A)" > "$d/AgentMux-$VER-envelope.json.sig"
    }
    run_install() { # <release dir> [extra args...]
        rel="$1"; shift
        rm -rf "$W/h" && mkdir -p "$W/h"
        HOME="$W/h" AGENTMUX_INSTALL_ROOT="$W/none" AGENTMUX_INSTALL_OS=Linux AGENTMUX_INSTALL_ARCH=x86_64 AGENTMUX_INSTALL_PM=none \
            AGENTMUX_INSTALL_TEST_KEYS="${KEYS:-$TESTKEYS}" AGENTMUX_INSTALL_RELEASE_BASE="$rel" sh "$SCRIPT" --no-launch --method download --version "$VER" "$@"
    }

    make_release "$W/good" "v$VER"
    expect "a good release installs" 0 "matches the signed manifest" run_install "$W/good"
    [ -x "$W/h/.local/share/agentmux/AgentMux/agentmux.sh" ] && [ -L "$W/h/.local/bin/agentmux" ] && pass "the portable copy and the agentmux link exist" || fail "the portable copy and the agentmux link exist"

    cp -r "$W/good" "$W/badsize"; printf 'x' >> "$W/badsize/$NAME"
    expect "a changed file is refused" 1 "bytes, the manifest says" run_install "$W/badsize"

    cp -r "$W/good" "$W/badhash"; size="$(wc -c < "$W/good/$NAME")"; head -c "$size" /dev/urandom > "$W/badhash/$NAME"
    expect "a same-size different file is refused" 1 "does not match the manifest's SHA-256" run_install "$W/badhash"

    cp -r "$W/good" "$W/badsig"; first="$(cut -c4 "$W/badsig/AgentMux-$VER-envelope.json.sig" | head -n 1)"; swap=A; [ "$first" = A ] && swap=B
    sed_inplace "s/^t1 ./t1 $swap/" "$W/badsig/AgentMux-$VER-envelope.json.sig"
    expect "a wrong signature is refused" 1 "signature does not match" run_install "$W/badsig"

    cp -r "$W/good" "$W/badmanifest"; sed_inplace 's/"format": 1/"format":  1/' "$W/badmanifest/AgentMux-$VER-manifest.json"
    expect "a changed manifest is refused" 1 "not the one the envelope names" run_install "$W/badmanifest"

    make_release "$W/othertag" "v1.2.3"
    expect "an envelope for another release is refused" 1 "another release" run_install "$W/othertag"

    make_release "$W/unlisted" "v$VER" "Other.tar.xz"
    expect "a file the signed manifest does not list is refused" 1 "does not list $NAME" run_install "$W/unlisted"

    # ---- the server package, from a release made here ----
    SNAME="agentmux-headless-${VER}-linux-x64.tar.gz"
    # make_server_release <dir> <tag> [what to leave out of the package: srv|web]
    make_server_release() {
        d="$1"; tag="$2"; leave="${3:-}"
        top="agentmux-headless-${VER}-linux-x64"
        rm -rf "$W/spkg"; mkdir -p "$d" "$W/spkg/$top/bin" "$W/spkg/$top/web"
        printf '#!/bin/sh\necho "the server program: $0 $*"\n' > "$W/spkg/$top/bin/agentmux"
        cp "$W/spkg/$top/bin/agentmux" "$W/spkg/$top/bin/agentmux-srv"
        chmod +x "$W/spkg/$top/bin/agentmux" "$W/spkg/$top/bin/agentmux-srv"
        printf '<html></html>\n' > "$W/spkg/$top/web/index.html"
        if [ "$leave" = srv ]; then rm "$W/spkg/$top/bin/agentmux-srv"; fi
        if [ "$leave" = web ]; then rm "$W/spkg/$top/web/index.html"; fi
        tar -czf "$d/$SNAME" -C "$W/spkg" "$top"
        sign_release "$d" "$tag" "$SNAME" "$SNAME"
    }
    run_server_install() { # <release dir> [extra args...]; the home is $W/hs, left in place for the checks
        rel="$1"; shift
        if [ -z "${KEEP_HS:-}" ]; then rm -rf "$W/hs"; fi
        mkdir -p "$W/hs"
        HOME="$W/hs" PATH="${SERVER_TEST_PATH:-$PATH}" AGENTMUX_INSTALL_ROOT="$W/none" AGENTMUX_INSTALL_OS=Linux AGENTMUX_INSTALL_ARCH=x86_64 \
            AGENTMUX_INSTALL_TEST_KEYS="$TESTKEYS" AGENTMUX_INSTALL_RELEASE_BASE="$rel" sh "$SCRIPT" --headless --version "$VER" "$@"
    }

    make_server_release "$W/sgood" "v$VER"
    expect "a good server release installs" 0 "matches the signed manifest" run_server_install "$W/sgood"
    if [ -x "$W/hs/.agentmux/headless/$VER/bin/agentmux" ] && [ -f "$W/hs/.agentmux/headless/$VER/web/index.html" ]; then pass "the server package is unpacked under its version"; else fail "the server package is unpacked under its version"; fi
    if [ "$(readlink "$W/hs/.agentmux/headless/current")" = "$W/hs/.agentmux/headless/$VER" ]; then pass "current points at the new version"; else fail "current points at the new version"; fi
    if [ -L "$W/hs/.local/bin/agentmux-headless" ] && [ -L "$W/hs/.local/bin/agentmux" ]; then pass "both names are linked when agentmux is free"; else fail "both names are linked when agentmux is free"; fi
    expect "the linked program runs from the install" 0 "the server program" "$W/hs/.local/bin/agentmux-headless" status
    KEEP_HS=1 expect "a second install over the first works" 0 "matches the signed manifest" run_server_install "$W/sgood"
    if [ -L "$W/hs/.local/bin/agentmux-headless" ] && [ -L "$W/hs/.local/bin/agentmux" ]; then pass "a reinstall keeps both links"; else fail "a reinstall keeps both links"; fi
    KEEP_HS=1 expect "a reinstall does not call its own agentmux link another program" 0 "next: run  agentmux setup" run_server_install "$W/sgood"

    # An install from before the rename (~/.agentmux/server, linked as agentmux-server and agentmux) is taken over:
    # the links move to the new program, and the old folder is named so it can be removed, not deleted.
    rm -rf "$W/hs"; mkdir -p "$W/hs/.agentmux/server/0.60.1/bin" "$W/hs/.local/bin"
    printf '#!/bin/sh\necho old\n' > "$W/hs/.agentmux/server/0.60.1/bin/agentmux"; chmod +x "$W/hs/.agentmux/server/0.60.1/bin/agentmux"
    ln -s "$W/hs/.agentmux/server/0.60.1" "$W/hs/.agentmux/server/current"
    ln -s "$W/hs/.agentmux/server/current/bin/agentmux" "$W/hs/.local/bin/agentmux-server"
    ln -s "$W/hs/.agentmux/server/current/bin/agentmux" "$W/hs/.local/bin/agentmux"
    KEEP_HS=1 expect "an install from before the rename is named, with how to remove it" 0 "from before the rename" run_server_install "$W/sgood"
    if [ "$(readlink "$W/hs/.local/bin/agentmux")" = "$W/hs/.agentmux/headless/current/bin/agentmux" ]; then pass "its agentmux link now leads to the new install"; else fail "its agentmux link now leads to the new install"; fi
    if [ ! -e "$W/hs/.local/bin/agentmux-server" ] && [ ! -L "$W/hs/.local/bin/agentmux-server" ]; then pass "the old agentmux-server link is gone"; else fail "the old agentmux-server link is gone"; fi
    if [ -f "$W/hs/.agentmux/server/0.60.1/bin/agentmux" ]; then pass "the old folder is left for the person to remove"; else fail "the old folder is left for the person to remove"; fi

    # A launcher already at ~/.local/bin/agentmux (the desktop's) is left alone even when that folder is not on the PATH.
    rm -rf "$W/hs"; mkdir -p "$W/hs/.local/bin"; printf '#!/bin/sh\necho desktop-launcher\n' > "$W/hs/.local/bin/agentmux"; chmod +x "$W/hs/.local/bin/agentmux"
    KEEP_HS=1 expect "a launcher at the link's path is named in the note" 0 "run this one as  agentmux-headless" run_server_install "$W/sgood"
    expect "the launcher at the link's path is untouched" 0 "desktop-launcher" "$W/hs/.local/bin/agentmux"
    if [ -L "$W/hs/.local/bin/agentmux-headless" ]; then pass "agentmux-headless is linked beside it"; else fail "agentmux-headless is linked beside it"; fi
    unset KEEP_HS

    # Something else named agentmux (the desktop app's launcher) is left alone.
    mkdir -p "$W/fakebin"; printf '#!/bin/sh\necho desktop\n' > "$W/fakebin/agentmux"; chmod +x "$W/fakebin/agentmux"
    SERVER_TEST_PATH="$W/fakebin:$PATH" expect "another agentmux on the PATH is named in the note" 0 "run this one as  agentmux-headless" run_server_install "$W/sgood"
    if [ -L "$W/hs/.local/bin/agentmux-headless" ] && [ ! -e "$W/hs/.local/bin/agentmux" ]; then pass "only agentmux-headless is linked then"; else fail "only agentmux-headless is linked then"; fi
    expect "the other agentmux is untouched" 0 "desktop" "$W/fakebin/agentmux"

    SERVER_TEST_PATH="$W/fakebin:$PATH" expect "--setup runs setup under the name that is free" 0 "the server program" run_server_install "$W/sgood" --setup
    cp -r "$W/sgood" "$W/sbad2"; printf 'x' >> "$W/sbad2/$SNAME"
    expect "a changed server package is refused" 1 "bytes, the manifest says" run_server_install "$W/sbad2"
    if [ ! -e "$W/hs/.agentmux/headless/$VER" ]; then pass "nothing is installed from a refused package"; else fail "nothing is installed from a refused package"; fi
    make_server_release "$W/snosrv" "v$VER" srv
    expect "a package with no server program is refused" 1 "is not an AgentMux Headless package" run_server_install "$W/snosrv"
    make_server_release "$W/snoweb" "v$VER" web
    expect "a package with no web UI is refused" 1 "is not an AgentMux Headless package" run_server_install "$W/snoweb"
    make_server_release "$W/sunlisted" "v$VER"; sign_release "$W/sunlisted" "v$VER" "Other.tar.gz" "$SNAME"
    expect "a server package the signed manifest does not list is refused" 1 "does not list $SNAME" run_server_install "$W/sunlisted"
    expect "a release with no package for this system says so" 1 "has no AgentMux Headless package for this system" run_server_install "$W/sunlisted"

    # ---- a system whose openssl cannot check Ed25519 (a stock Mac's LibreSSL 3.3.6): which check ran, and what --require-signature does ----
    mkdir -p "$W/libre"
    printf '#!/bin/sh\ncase "$1" in\n    version) echo "LibreSSL 3.3.6" ;;\n    pkeyutl) echo "usage: pkeyutl [-in file] [-inkey file] [-sign] [-verify]" >&2; exit 1 ;;\n    base64) exec "%s" "$@" ;;\n    *) exit 1 ;;\nesac\n' "$(command -v openssl)" > "$W/libre/openssl"
    chmod +x "$W/libre/openssl"
    # A system whose ssh-keygen has no -Y (OpenSSH before 8.0) as well: the hash-only case, the one these checks are about.
    mkdir -p "$W/oldssh"; printf '#!/bin/sh\necho "ssh-keygen: illegal option -- Y" >&2\nexit 1\n' > "$W/oldssh/ssh-keygen"; chmod +x "$W/oldssh/ssh-keygen"
    run_libre() { PATH="$W/oldssh:$W/libre:$PATH" AGENTMUX_INSTALL_OPENSSL_PLACES="${PLACES:-$W/nowhere}" run_install "$@"; }
    expect "LibreSSL: installs on the hashes and says the signature was NOT checked" 0 "signature NOT checked" run_libre "$W/good"
    expect "LibreSSL: names the openssl that cannot" 0 "LibreSSL 3.3.6" run_libre "$W/good"
    expect "LibreSSL: says what was checked instead" 0 "size and SHA-256" run_libre "$W/good"
    expect "LibreSSL: the final line does not claim a signed manifest" 0 "the manifest's signature was not checked" run_libre "$W/good"
    refuses_to_contain "LibreSSL: never says the signature was checked" "signature of the release checked" run_libre "$W/good"
    expect "LibreSSL: --require-signature refuses" 1 "cannot be checked here (LibreSSL 3.3.6" run_libre "$W/good" --require-signature
    expect "LibreSSL: a changed file is still refused" 1 "does not match the manifest's SHA-256" run_libre "$W/badhash"
    PLACES="$(command -v openssl)" expect "LibreSSL first on the PATH, an OpenSSL 3 in the usual places: the signature is checked" 0 "signature of the release checked: Ed25519, with OpenSSL 3" run_libre "$W/good"
    PLACES="$(command -v openssl)" expect "that check is also what --require-signature wants" 0 "signature of the release checked" run_libre "$W/good" --require-signature
    PLACES="$(command -v openssl)" expect "and a wrong signature is refused there" 1 "signature does not match" run_libre "$W/badsig"
    expect "the real openssl: the message names Ed25519 and the version" 0 "signature of the release checked: Ed25519, with OpenSSL 3" run_install "$W/good"

    # ---- the SSH signature of the same envelope, checked with the system's ssh-keygen (what a stock Mac can do) ----
    if [ -n "${CI:-}" ] && ! command -v ssh-keygen > /dev/null 2>&1; then fail "ssh-keygen is installed where CI runs (a skip here would hide these checks)"; fi
    if ! command -v ssh-keygen > /dev/null 2>&1; then
        echo "skip - no ssh-keygen: the SSH signature checks are not run"
    else
        ssh-keygen -q -t ed25519 -N '' -f "$W/sshA" -C a && ssh-keygen -q -t ed25519 -N '' -f "$W/sshB" -C b
        rawkey() { awk '{print $2}' "$1.pub" | openssl base64 -d -A | tail -c 32 | openssl base64 -A; }
        KEYA="t2:$(rawkey "$W/sshA")"
        # sign_ssh <release dir> <key> [namespace]: the envelope's SSH signature as AgentMux-<v>-envelope.json.sshsig
        sign_ssh() {
            cp "$1/AgentMux-$VER-envelope.json" "$W/ev.json"
            rm -f "$W/ev.json.sig"
            ssh-keygen -q -Y sign -f "$2" -n "${3:-agentmux-release}" "$W/ev.json" && mv "$W/ev.json.sig" "$1/AgentMux-$VER-envelope.json.sshsig"
        }
        run_ssh() { KEYS="$KEYA" PATH="$W/libre:$PATH" AGENTMUX_INSTALL_OPENSSL_PLACES="$W/nowhere" run_install "$@"; }

        rm -rf "$W/ssh-good" && cp -r "$W/good" "$W/ssh-good" && sign_ssh "$W/ssh-good" "$W/sshA"
        expect "SSH signature: a stock-Mac-like system (LibreSSL) checks the release with ssh-keygen" 0 "signature of the release checked: Ed25519 (SSH signature), with " run_ssh "$W/ssh-good"
        expect "SSH signature: the install goes on to match the manifest" 0 "matches the signed manifest" run_ssh "$W/ssh-good"
        refuses_to_contain "SSH signature: nothing says the signature was not checked" "NOT checked" run_ssh "$W/ssh-good"
        expect "SSH signature: --require-signature passes with it" 0 "signature of the release checked" run_ssh "$W/ssh-good" --require-signature

        cp -r "$W/ssh-good" "$W/ssh-changed" && printf ' ' >> "$W/ssh-changed/AgentMux-$VER-envelope.json"
        expect "SSH signature: a changed envelope is refused" 1 "signature does not match" run_ssh "$W/ssh-changed"

        rm -rf "$W/ssh-ns" && cp -r "$W/good" "$W/ssh-ns" && sign_ssh "$W/ssh-ns" "$W/sshA" "another-namespace"
        expect "SSH signature: another namespace is refused" 1 "signature does not match" run_ssh "$W/ssh-ns"

        rm -rf "$W/ssh-other" && cp -r "$W/good" "$W/ssh-other" && sign_ssh "$W/ssh-other" "$W/sshB"
        expect "SSH signature: a good signature by a key that is not pinned is refused (the key inside it is never trusted)" 1 "signature does not match" run_ssh "$W/ssh-other"
        expect "SSH signature: it is accepted when that key is the pinned one" 0 "signature of the release checked" env KEYS="t2:$(rawkey "$W/sshB")" PATH="$W/libre:$PATH" AGENTMUX_INSTALL_OPENSSL_PLACES="$W/nowhere" AGENTMUX_INSTALL_TEST_KEYS="t2:$(rawkey "$W/sshB")" HOME="$W/h3" AGENTMUX_INSTALL_ROOT="$W/none" AGENTMUX_INSTALL_OS=Linux AGENTMUX_INSTALL_ARCH=x86_64 AGENTMUX_INSTALL_PM=none AGENTMUX_INSTALL_RELEASE_BASE="$W/ssh-other" sh "$SCRIPT" --no-launch --method download --version "$VER"

        # No quiet downgrade: from the floor on, a release without the SSH signature is refused where only that check is possible.
        expect "SSH signature: from the floor on, a release without one is refused, not installed on hashes" 1 "should carry AgentMux-$VER-envelope.json.sshsig" run_ssh "$W/good"
        refuses_to_contain "SSH signature: the refusal is not a hash-only install" "matches the manifest" run_ssh "$W/good"
        # A suffix on the version is ignored: 9.9.9-rc1 is at the floor 9.9.9 (never below it), and below 9.9.10.
        OLDVER="$VER"; OLDNAME="$NAME"; VER="9.9.9-rc1"; NAME="AgentMux_${VER}_amd64-portable.tar.xz"
        make_release "$W/rc" "v$VER"
        AGENTMUX_INSTALL_SSHSIG_FLOOR=9.9.9 expect "SSH signature: a version with a suffix is at the floor, not below it" 1 "should carry AgentMux-$VER-envelope.json.sshsig" run_ssh "$W/rc"
        AGENTMUX_INSTALL_SSHSIG_FLOOR=9.9.10 expect "SSH signature: and below the next floor it is hashes only" 0 "signature NOT checked" run_ssh "$W/rc"
        VER="$OLDVER"; NAME="$OLDNAME"
        # Below the floor (a release from before the first one that carries it): today's behaviour, said plainly.
        AGENTMUX_INSTALL_SSHSIG_FLOOR=99.0.0 expect "SSH signature: below the floor, hashes only, and it says so" 0 "signature NOT checked" run_ssh "$W/good"
        AGENTMUX_INSTALL_SSHSIG_FLOOR=99.0.0 expect "SSH signature: below the floor, it says the release is older than the first with an SSH signature, not that ssh-keygen is missing" 0 "is older than 99.0.0, the first with an SSH signature" run_ssh "$W/good"
        AGENTMUX_INSTALL_SSHSIG_FLOOR=99.0.0 refuses_to_contain "SSH signature: below the floor it does not blame ssh-keygen" "no ssh-keygen of OpenSSH 8.0 or newer" run_ssh "$W/good"
        AGENTMUX_INSTALL_SSHSIG_FLOOR=99.0.0 expect "SSH signature: below the floor, --require-signature refuses" 1 "cannot be checked here" run_ssh "$W/good" --require-signature
        # An ssh-keygen with no -Y (OpenSSH before 8.0) cannot check: hashes only, and --require-signature refuses.
        run_oldssh() { KEYS="$KEYA" PATH="$W/oldssh:$W/libre:$PATH" AGENTMUX_INSTALL_OPENSSL_PLACES="$W/nowhere" run_install "$@"; }
        expect "SSH signature: an old OpenSSH (no -Y) falls back to hashes and says so" 0 "signature NOT checked" run_oldssh "$W/good"
        expect "SSH signature: an old OpenSSH is named as the reason" 0 "no ssh-keygen of OpenSSH 8.0 or newer" run_oldssh "$W/good"
        expect "SSH signature: an old OpenSSH is not blamed for a missing .sshsig" 0 "matches the manifest" run_oldssh "$W/good"
        expect "SSH signature: an old OpenSSH with --require-signature is refused" 1 "OpenSSH 8.0 or newer" run_oldssh "$W/good" --require-signature
        # With OpenSSL 3 the existing check is used first and needs no .sshsig.
        expect "SSH signature: with OpenSSL 3 the .sig is checked and no .sshsig is needed" 0 "signature of the release checked: Ed25519, with OpenSSL 3" run_install "$W/good"
    fi

    # ---- curl with no CA certificates ----
    for code in 60 77 6; do mkdir -p "$W/curl$code"; printf '#!/bin/sh\nexit %s\n' "$code" > "$W/curl$code/curl"; chmod +x "$W/curl$code/curl"; done
    run_nonet() { # <dir with a fake curl first on the PATH> [args...]
        d="$1"; shift
        HOME="$W/h2" PATH="$d:$PATH" AGENTMUX_INSTALL_ROOT="$W/none" AGENTMUX_INSTALL_OS=Linux AGENTMUX_INSTALL_ARCH=x86_64 AGENTMUX_INSTALL_PM=none \
            sh "$SCRIPT" --no-launch --method download "$@"
    }
    expect "curl error 77 looking up the release: install ca-certificates" 1 "ca-certificates package is not installed" run_nonet "$W/curl77"
    expect "curl error 77 names the lookup that failed" 1 "could not look up the latest release" run_nonet "$W/curl77"
    expect "curl error 60 looking up the release: install ca-certificates" 1 "sudo apt-get install ca-certificates" run_nonet "$W/curl60"
    expect "curl error 77 while downloading: install ca-certificates" 1 "ca-certificates package is not installed" run_nonet "$W/curl77" --version "$VER"
    expect "curl error 60 while downloading names the file" 1 "could not download https://github.com/agentmuxai/agentmux/releases/download/v$VER/AgentMux-$VER-manifest.json" run_nonet "$W/curl60" --version "$VER"
    refuses_to_contain "another curl failure (6, no such host) is not blamed on certificates" "ca-certificates" run_nonet "$W/curl6"
    expect "another curl failure keeps its plain message" 1 "could not look up the latest release" run_nonet "$W/curl6"

    # ---- after an install into ~/.local/bin: where it is, how to run it now, how to put it on the PATH ----
    expect "the portable install gives the full path to run now" 0 "run it now with the full path:  $W/h/.local/bin/agentmux" run_install "$W/good"
    expect "the portable install gives the line that adds it to this shell's PATH" 0 'export PATH="$HOME/.local/bin:$PATH"' run_install "$W/good"
    PATH="$W/h/.local/bin:$PATH" refuses_to_contain "no PATH hint when the folder is already on the PATH" "export PATH" run_install "$W/good"
    expect "the server install gives the full path to run now" 0 "run it now with the full path:  $W/hs/.local/bin/agentmux" run_server_install "$W/sgood"
    expect "the server install gives the PATH line too" 0 'export PATH="$HOME/.local/bin:$PATH"' run_server_install "$W/sgood"

    # With the app's real key the test signature must not verify.
    expect "another key's signature is refused" 1 "signature does not match" env AGENTMUX_INSTALL_TEST_KEYS="k1:m9HRZ4UOiBU2vyrjlHXj1D0CjfhAazI2DGdII9JSJLE=" HOME="$W/h" AGENTMUX_INSTALL_ROOT="$W/none" AGENTMUX_INSTALL_OS=Linux AGENTMUX_INSTALL_ARCH=x86_64 AGENTMUX_INSTALL_PM=none AGENTMUX_INSTALL_RELEASE_BASE="$W/good" sh "$SCRIPT" --no-launch --method download --version "$VER"
fi

echo
if [ "$FAILS" = 0 ]; then echo "all passed"; else echo "$FAILS failed"; exit 1; fi

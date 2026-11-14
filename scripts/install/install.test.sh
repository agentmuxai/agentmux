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
    AGENTMUX_INSTALL_PM=apt expect "server dry run, linux x64" 0 "agentmux-server-0.60.14-linux-x64.tar.gz" "$SH" "$SCRIPT" --server --dry-run --version 0.60.14
    AGENTMUX_INSTALL_ARCH=aarch64 expect "server dry run, linux arm64" 0 "agentmux-server-0.60.14-linux-arm64.tar.gz" "$SH" "$SCRIPT" --server --dry-run --version 0.60.14
    AGENTMUX_INSTALL_OS=Darwin AGENTMUX_INSTALL_ARCH=arm64 expect "server dry run, macos arm64" 0 "agentmux-server-0.60.14-macos-arm64.tar.gz" "$SH" "$SCRIPT" --server --dry-run --version 0.60.14
    AGENTMUX_INSTALL_OS=Darwin AGENTMUX_INSTALL_ARCH=x86_64 expect "server dry run, macos x64" 0 "agentmux-server-0.60.14-macos-x64.tar.gz" "$SH" "$SCRIPT" --server --dry-run --version 0.60.14
    expect "server dry run links the program under both names" 0 "/.local/bin/agentmux-server" "$SH" "$SCRIPT" --server --dry-run --version 0.60.14
    expect "server dry run with --setup runs setup" 0 "agentmux-server setup --yes --no-open" "$SH" "$SCRIPT" --server --setup --dry-run --version 0.60.14
    AGENTMUX_INSTALL_ROOT="$W/root-linux" AGENTMUX_INSTALL_PM=apt expect "an installed desktop app does not stop the server install" 0 "agentmux-server-0.60.14-linux-x64.tar.gz" "$SH" "$SCRIPT" --server --dry-run --version 0.60.14
    expect "the server has no package-manager method" 1 "no package-manager method" "$SH" "$SCRIPT" --server --method package-manager --dry-run
    expect "--setup is for --server" 1 "--setup is for --server" "$SH" "$SCRIPT" --setup --dry-run
    AGENTMUX_INSTALL_OS=FreeBSD expect "the server has no build for an unknown system" 1 "unsupported system" "$SH" "$SCRIPT" --server --dry-run --version 0.60.14

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
        sha="$(sha256sum < "$d/$file" | cut -d ' ' -f 1)"
        # The same shape release-manifest.mjs writes: indented JSON, each file as name, size, sha256.
        printf '{\n  "format": 1,\n  "version": "%s",\n  "tag": "%s",\n  "files": [\n    {\n      "name": "%s",\n      "size": %s,\n      "sha256": "%s"\n    }\n  ]\n}\n' "$VER" "$tag" "$listed" "$size" "$sha" > "$d/AgentMux-$VER-manifest.json"
        msha="$(sha256sum < "$d/AgentMux-$VER-manifest.json" | cut -d ' ' -f 1)"
        printf '{\n  "format": 1,\n  "tag": "%s",\n  "manifest_sha256": "%s",\n  "created_at": "2026-11-04T00:00:00Z"\n}\n' "$tag" "$msha" > "$d/AgentMux-$VER-envelope.json"
        printf 't1 %s\n' "$(openssl pkeyutl -sign -inkey "$W/priv.pem" -rawin -in "$d/AgentMux-$VER-envelope.json" | openssl base64 -A)" > "$d/AgentMux-$VER-envelope.json.sig"
    }
    run_install() { # <release dir> [extra args...]
        rel="$1"; shift
        rm -rf "$W/h" && mkdir -p "$W/h"
        HOME="$W/h" AGENTMUX_INSTALL_ROOT="$W/none" AGENTMUX_INSTALL_OS=Linux AGENTMUX_INSTALL_ARCH=x86_64 AGENTMUX_INSTALL_PM=none \
            AGENTMUX_INSTALL_TEST_KEYS="$TESTKEYS" AGENTMUX_INSTALL_RELEASE_BASE="$rel" sh "$SCRIPT" --no-launch --method download --version "$VER" "$@"
    }

    make_release "$W/good" "v$VER"
    expect "a good release installs" 0 "matches the signed manifest" run_install "$W/good"
    [ -x "$W/h/.local/share/agentmux/AgentMux/agentmux.sh" ] && [ -L "$W/h/.local/bin/agentmux" ] && pass "the portable copy and the agentmux link exist" || fail "the portable copy and the agentmux link exist"

    cp -r "$W/good" "$W/badsize"; printf 'x' >> "$W/badsize/$NAME"
    expect "a changed file is refused" 1 "bytes, the manifest says" run_install "$W/badsize"

    cp -r "$W/good" "$W/badhash"; size="$(wc -c < "$W/good/$NAME")"; head -c "$size" /dev/urandom > "$W/badhash/$NAME"
    expect "a same-size different file is refused" 1 "does not match the manifest's SHA-256" run_install "$W/badhash"

    cp -r "$W/good" "$W/badsig"; first="$(cut -c4 "$W/badsig/AgentMux-$VER-envelope.json.sig" | head -n 1)"; swap=A; [ "$first" = A ] && swap=B
    sed -i "s/^t1 ./t1 $swap/" "$W/badsig/AgentMux-$VER-envelope.json.sig"
    expect "a wrong signature is refused" 1 "signature does not match" run_install "$W/badsig"

    cp -r "$W/good" "$W/badmanifest"; sed -i 's/"format": 1/"format":  1/' "$W/badmanifest/AgentMux-$VER-manifest.json"
    expect "a changed manifest is refused" 1 "not the one the envelope names" run_install "$W/badmanifest"

    make_release "$W/othertag" "v1.2.3"
    expect "an envelope for another release is refused" 1 "another release" run_install "$W/othertag"

    make_release "$W/unlisted" "v$VER" "Other.tar.xz"
    expect "a file the signed manifest does not list is refused" 1 "does not list $NAME" run_install "$W/unlisted"

    # ---- the server package, from a release made here ----
    SNAME="agentmux-server-${VER}-linux-x64.tar.gz"
    # make_server_release <dir> <tag> [what to leave out of the package: srv|web]
    make_server_release() {
        d="$1"; tag="$2"; leave="${3:-}"
        top="agentmux-server-${VER}-linux-x64"
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
            AGENTMUX_INSTALL_TEST_KEYS="$TESTKEYS" AGENTMUX_INSTALL_RELEASE_BASE="$rel" sh "$SCRIPT" --server --version "$VER" "$@"
    }

    make_server_release "$W/sgood" "v$VER"
    expect "a good server release installs" 0 "matches the signed manifest" run_server_install "$W/sgood"
    if [ -x "$W/hs/.agentmux/server/$VER/bin/agentmux" ] && [ -f "$W/hs/.agentmux/server/$VER/web/index.html" ]; then pass "the server package is unpacked under its version"; else fail "the server package is unpacked under its version"; fi
    if [ "$(readlink "$W/hs/.agentmux/server/current")" = "$W/hs/.agentmux/server/$VER" ]; then pass "current points at the new version"; else fail "current points at the new version"; fi
    if [ -L "$W/hs/.local/bin/agentmux-server" ] && [ -L "$W/hs/.local/bin/agentmux" ]; then pass "both names are linked when agentmux is free"; else fail "both names are linked when agentmux is free"; fi
    expect "the linked program runs from the install" 0 "the server program" "$W/hs/.local/bin/agentmux-server" status
    KEEP_HS=1 expect "a second install over the first works" 0 "matches the signed manifest" run_server_install "$W/sgood"
    if [ -L "$W/hs/.local/bin/agentmux-server" ] && [ -L "$W/hs/.local/bin/agentmux" ]; then pass "a reinstall keeps both links"; else fail "a reinstall keeps both links"; fi
    KEEP_HS=1 expect "a reinstall does not call its own agentmux link another program" 0 "next: run  agentmux setup" run_server_install "$W/sgood"

    # A launcher already at ~/.local/bin/agentmux (the desktop's) is left alone even when that folder is not on the PATH.
    rm -rf "$W/hs"; mkdir -p "$W/hs/.local/bin"; printf '#!/bin/sh\necho desktop-launcher\n' > "$W/hs/.local/bin/agentmux"; chmod +x "$W/hs/.local/bin/agentmux"
    KEEP_HS=1 expect "a launcher at the link's path is named in the note" 0 "run this one as  agentmux-server" run_server_install "$W/sgood"
    expect "the launcher at the link's path is untouched" 0 "desktop-launcher" "$W/hs/.local/bin/agentmux"
    if [ -L "$W/hs/.local/bin/agentmux-server" ]; then pass "agentmux-server is linked beside it"; else fail "agentmux-server is linked beside it"; fi
    unset KEEP_HS

    # Something else named agentmux (the desktop app's launcher) is left alone.
    mkdir -p "$W/fakebin"; printf '#!/bin/sh\necho desktop\n' > "$W/fakebin/agentmux"; chmod +x "$W/fakebin/agentmux"
    SERVER_TEST_PATH="$W/fakebin:$PATH" expect "another agentmux on the PATH is named in the note" 0 "run this one as  agentmux-server" run_server_install "$W/sgood"
    if [ -L "$W/hs/.local/bin/agentmux-server" ] && [ ! -e "$W/hs/.local/bin/agentmux" ]; then pass "only agentmux-server is linked then"; else fail "only agentmux-server is linked then"; fi
    expect "the other agentmux is untouched" 0 "desktop" "$W/fakebin/agentmux"

    SERVER_TEST_PATH="$W/fakebin:$PATH" expect "--setup runs setup under the name that is free" 0 "the server program" run_server_install "$W/sgood" --setup
    cp -r "$W/sgood" "$W/sbad2"; printf 'x' >> "$W/sbad2/$SNAME"
    expect "a changed server package is refused" 1 "bytes, the manifest says" run_server_install "$W/sbad2"
    if [ ! -e "$W/hs/.agentmux/server/$VER" ]; then pass "nothing is installed from a refused package"; else fail "nothing is installed from a refused package"; fi
    make_server_release "$W/snosrv" "v$VER" srv
    expect "a package with no server program is refused" 1 "is not an AgentMux server package" run_server_install "$W/snosrv"
    make_server_release "$W/snoweb" "v$VER" web
    expect "a package with no web UI is refused" 1 "is not an AgentMux server package" run_server_install "$W/snoweb"
    make_server_release "$W/sunlisted" "v$VER"; sign_release "$W/sunlisted" "v$VER" "Other.tar.gz" "$SNAME"
    expect "a server package the signed manifest does not list is refused" 1 "does not list $SNAME" run_server_install "$W/sunlisted"
    expect "a release with no package for this system says so" 1 "has no server package for this system" run_server_install "$W/sunlisted"

    # With the app's real key the test signature must not verify.
    expect "another key's signature is refused" 1 "signature does not match" env AGENTMUX_INSTALL_TEST_KEYS="k1:m9HRZ4UOiBU2vyrjlHXj1D0CjfhAazI2DGdII9JSJLE=" HOME="$W/h" AGENTMUX_INSTALL_ROOT="$W/none" AGENTMUX_INSTALL_OS=Linux AGENTMUX_INSTALL_ARCH=x86_64 AGENTMUX_INSTALL_PM=none AGENTMUX_INSTALL_RELEASE_BASE="$W/good" sh "$SCRIPT" --no-launch --method download --version "$VER"
fi

echo
if [ "$FAILS" = 0 ]; then echo "all passed"; else echo "$FAILS failed"; exit 1; fi

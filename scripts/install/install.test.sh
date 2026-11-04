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

    # make_release <dir> <tag> : a tar.xz with an AgentMux/agentmux.sh, its manifest, the envelope and the signature
    make_release() {
        d="$1"; tag="$2"
        mkdir -p "$d" "$W/pkg/AgentMux"
        printf '#!/bin/sh\necho hello\n' > "$W/pkg/AgentMux/agentmux.sh"; chmod +x "$W/pkg/AgentMux/agentmux.sh"
        tar -cJf "$d/$NAME" -C "$W/pkg" AgentMux
        size="$(wc -c < "$d/$NAME" | tr -d ' ')"
        sha="$(sha256sum "$d/$NAME" | cut -d ' ' -f 1)"
        # The same shape release-manifest.mjs writes: indented JSON, each file as name, size, sha256.
        printf '{\n  "format": 1,\n  "version": "%s",\n  "tag": "%s",\n  "files": [\n    {\n      "name": "%s",\n      "size": %s,\n      "sha256": "%s"\n    }\n  ]\n}\n' "$VER" "$tag" "$NAME" "$size" "$sha" > "$d/AgentMux-$VER-manifest.json"
        msha="$(sha256sum "$d/AgentMux-$VER-manifest.json" | cut -d ' ' -f 1)"
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

    cp -r "$W/good" "$W/badsig"; sed -i 's/^t1 ./t1 A/' "$W/badsig/AgentMux-$VER-envelope.json.sig"
    expect "a wrong signature is refused" 1 "signature does not match" run_install "$W/badsig"

    cp -r "$W/good" "$W/badmanifest"; sed -i 's/"format": 1/"format":  1/' "$W/badmanifest/AgentMux-$VER-manifest.json"
    expect "a changed manifest is refused" 1 "not the one the envelope names" run_install "$W/badmanifest"

    make_release "$W/othertag" "v1.2.3"
    expect "an envelope for another release is refused" 1 "another release" run_install "$W/othertag"

    cp -r "$W/good" "$W/unlisted"; sed -i "s/$NAME/Other.tar.xz/" "$W/unlisted/AgentMux-$VER-manifest.json"
    expect "a file the manifest does not list is refused" 1 "not the one the envelope names" run_install "$W/unlisted"

    # With the app's real key the test signature must not verify.
    expect "another key's signature is refused" 1 "signature does not match" env AGENTMUX_INSTALL_TEST_KEYS="k1:m9HRZ4UOiBU2vyrjlHXj1D0CjfhAazI2DGdII9JSJLE=" HOME="$W/h" AGENTMUX_INSTALL_ROOT="$W/none" AGENTMUX_INSTALL_OS=Linux AGENTMUX_INSTALL_ARCH=x86_64 AGENTMUX_INSTALL_PM=none AGENTMUX_INSTALL_RELEASE_BASE="$W/good" sh "$SCRIPT" --no-launch --method download --version "$VER"
fi

echo
if [ "$FAILS" = 0 ]; then echo "all passed"; else echo "$FAILS failed"; exit 1; fi

#!/bin/sh
# agentmux-install: install AgentMux on Linux or macOS from a terminal.
#
#   curl -fsSL https://agentmux.ai/install.sh | sh
#
# It uses the system's own package manager when there is one, so updates come from it:
#   Linux  apt, dnf or zypper: adds the signed repository at packages.agentmux.ai, installs `agentmux`
#   macOS  Homebrew (once the agentmuxai/homebrew-tap cask exists): brew install --cask agentmuxai/tap/agentmux
# and otherwise downloads a release file and checks it against the release's signed manifest:
#   Linux  the portable tarball, into ~/.local/share/agentmux (no root)
#   macOS  the .dmg, copied to /Applications (or ~/Applications)
#
# With --server it installs the AgentMux server package (no desktop app: a server whose web UI is a browser on its own
# loopback address) from the same signed release, into ~/.agentmux/server, on Linux x64 or arm64 and macOS arm64 or x64.
#
# Plain text, no prompts, ASCII only. Run it with --help for the options.
# Design: docs/specs/SPEC_ONE_COMMAND_INSTALL_ALL_PLATFORMS_2026_11_01.md
# The server package: docs/specs/SPEC_HEADLESS_SERVER_PACKAGE_AND_LOCAL_WEB_UI_2026_11_12.md
set -eu

SCRIPT_VERSION="1"
REPO="agentmuxai/agentmux"
RELEASES="https://github.com/$REPO/releases"
PKG_HOST="https://packages.agentmux.ai"
TAP_CASK_URL="https://raw.githubusercontent.com/agentmuxai/homebrew-tap/main/Casks/agentmux.rb"
# The public keys the app pins to verify a release (crates/common/update-keys.json), as "<id>:<base64 of the 32 raw bytes>".
# scripts/install/install.test.sh fails when this and that file disagree.
UPDATE_KEYS="k1:m9HRZ4UOiBU2vyrjlHXj1D0CjfhAazI2DGdII9JSJLE="

DRY_RUN=0
FORCE=0
NO_LAUNCH=0
REQUIRE_SIGNATURE=0
SERVER=0
SETUP=0
METHOD=auto
VERSION=""
TMP=""

say() { printf 'agentmux-install: %s\n' "$*"; }
die() { printf 'agentmux-install: error: %s\n' "$*" >&2; exit 1; }
# Print a command, then run it unless this is a dry run.
run() {
    printf '+ %s\n' "$*"
    if [ "$DRY_RUN" = 0 ]; then "$@" < /dev/null; fi
}
cleanup() { if [ -n "$TMP" ]; then rm -rf "$TMP"; fi; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

usage() {
    cat <<'EOF'
agentmux-install: install AgentMux on Linux or macOS.

Usage: install.sh [options]

  --version <x.y.z>         install this release (default: the latest)
  --method <m>              auto (default), package-manager or download
  --dry-run                 print what it would do and change nothing
  --no-launch               do not start AgentMux when done
  --force                   install even if AgentMux is already installed
  --require-signature       fail when the release's signature cannot be checked here
  --server                  install the AgentMux server (no desktop app) into ~/.agentmux/server, from the signed release
  --setup                   with --server: then run  agentmux setup --yes --no-open  (a user service that starts at login)
  --help                    this text

Environment: AGENTMUX_INSTALL_ROOT is a prefix for the paths it looks at (used by its tests).
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || die "--version needs a value"; VERSION="$2"; shift 2 ;;
        --version=*) VERSION="${1#--version=}"; shift ;;
        --method) [ $# -ge 2 ] || die "--method needs a value"; METHOD="$2"; shift 2 ;;
        --method=*) METHOD="${1#--method=}"; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        --no-launch) NO_LAUNCH=1; shift ;;
        --force) FORCE=1; shift ;;
        --require-signature) REQUIRE_SIGNATURE=1; shift ;;
        --server) SERVER=1; shift ;;
        --setup) SETUP=1; shift ;;
        --interactive) shift ;; # accepted and ignored: this script never prompts
        --help | -h) usage; exit 0 ;;
        *) die "unknown option: $1 (try --help)" ;;
    esac
done

case "$METHOD" in auto | package-manager | download) ;; *) die "--method is auto, package-manager or download" ;; esac
if [ "$SERVER" = 1 ] && [ "$METHOD" = package-manager ]; then die "--server installs from the release download: it has no package-manager method"; fi
if [ "$SETUP" = 1 ] && [ "$SERVER" = 0 ]; then die "--setup is for --server"; fi
if [ -n "$VERSION" ]; then
    printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.]+)?$' || die "--version must look like 0.60.14"
fi

ROOT="${AGENTMUX_INSTALL_ROOT:-}"

# ---- the platform ----
OS_RAW="${AGENTMUX_INSTALL_OS:-$(uname -s)}"
case "$OS_RAW" in
    Linux) OS=linux ;;
    Darwin) OS=macos ;;
    MINGW* | MSYS* | CYGWIN*) die "this is Windows: run  irm https://agentmux.ai/install.ps1 | iex  in PowerShell" ;;
    *) die "unsupported system: $OS_RAW" ;;
esac
ARCH_RAW="${AGENTMUX_INSTALL_ARCH:-$(uname -m)}"
case "$ARCH_RAW" in
    x86_64 | amd64) ARCH=x86_64 ;;
    arm64 | aarch64) ARCH=arm64 ;;
    *) die "unsupported processor: $ARCH_RAW" ;;
esac
# The desktop app has no arm64 Linux or Intel Mac build; the server package has all four.
if [ "$SERVER" = 0 ] && [ "$OS" = linux ] && [ "$ARCH" != x86_64 ]; then die "there is no arm64 Linux build yet"; fi
if [ "$SERVER" = 0 ] && [ "$OS" = macos ] && [ "$ARCH" != arm64 ]; then die "there is no Intel Mac build: AgentMux for macOS needs Apple Silicon"; fi

# ---- tools ----
have() { command -v "$1" > /dev/null 2>&1; }

fetch() { # fetch <url or absolute path> <file>
    case "$1" in
        /*) cp "$1" "$2" ;; # a local file: the tests' fixtures
        *)
            if have curl; then curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1" || die "could not download $1"
            elif have wget; then wget -q --https-only -O "$2" "$1" || die "could not download $1"
            else die "curl or wget is needed"
            fi
            ;;
    esac
}

sha256_of() {
    if have sha256sum; then sha256sum < "$1" | cut -d ' ' -f 1
    elif have shasum; then shasum -a 256 < "$1" | cut -d ' ' -f 1
    elif have openssl; then openssl dgst -sha256 < "$1" | sed 's/^.*= //'
    else die "sha256sum, shasum or openssl is needed to check the download"
    fi
}

SUDO=""
need_root_tool() {
    if [ "$(id -u)" != 0 ]; then
        if [ "$DRY_RUN" = 0 ]; then have sudo || die "this needs root and sudo is not installed: run it as root"; fi
        SUDO="sudo"
    fi
}

mktmp() { if [ -z "$TMP" ]; then TMP="$(mktemp -d)"; fi; }

# Put a file from the package host in a system folder.
fetch_to_system() { # <url> <destination>
    printf '+ fetch %s -> %s\n' "$1" "$2"
    if [ "$DRY_RUN" = 0 ]; then
        mktmp
        fetch "$1" "$TMP/fetched"
        $SUDO install -D -m 0644 "$TMP/fetched" "$2"
    fi
}

# ---- is it already installed? ----
installed_how() {
    if [ "$OS" = linux ]; then
        if have dpkg-query && dpkg-query -W -f='${Status}' agentmux 2> /dev/null | grep -q 'install ok installed'; then echo "an apt/dpkg package"; return; fi
        if have rpm && rpm -q agentmux > /dev/null 2>&1; then echo "an rpm package"; return; fi
        if [ -d "$ROOT/opt/agentmux" ]; then echo "a package or portable copy in /opt/agentmux"; return; fi
        if [ -x "$ROOT$HOME/.local/share/agentmux/AgentMux/agentmux.sh" ]; then echo "the portable copy in ~/.local/share/agentmux"; return; fi
    else
        if [ -d "$ROOT/Applications/AgentMux.app" ]; then echo "/Applications/AgentMux.app"; return; fi
        if [ -d "$ROOT$HOME/Applications/AgentMux.app" ]; then echo "~/Applications/AgentMux.app"; return; fi
    fi
}

ALREADY=""
if [ "$SERVER" = 0 ]; then ALREADY="$(installed_how || true)"; fi
if [ -n "$ALREADY" ] && [ "$FORCE" = 0 ]; then
    say "AgentMux is already installed ($ALREADY). It updates the way it was installed: from its version panel, or your package manager."
    say "Nothing changed. Use --force to install anyway."
    exit 0
fi

# ---- the release, and checking what is downloaded ----
resolve_version() {
    if [ -n "$VERSION" ]; then return; fi
    have curl || die "curl is needed to find the latest release (or pass --version)"
    url="$(curl -fsSLI --proto '=https' -o /dev/null -w '%{url_effective}' "$RELEASES/latest")" || die "could not look up the latest release"
    VERSION="${url##*/v}"
    printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.]+)?$' || die "the latest release's address ($url) is not a version"
}

# True when this openssl can check an Ed25519 signature in one shot (OpenSSL 3).
can_check_signature() { have openssl && openssl pkeyutl -help 2>&1 | grep -q -- '-rawin'; }

# verify_signature <envelope> <signature file> -> 0 checked and good, 1 checked and bad, 2 cannot check
verify_signature() {
    can_check_signature || return 2
    keys="${AGENTMUX_INSTALL_TEST_KEYS:-$UPDATE_KEYS}"
    for entry in $keys; do
        id="${entry%%:*}"
        b64="${entry#*:}"
        line="$(grep "^$id " "$2" | head -n 1 || true)"
        [ -n "$line" ] || continue
        printf '%s' "${line#* }" | openssl base64 -d -A > "$TMP/sig.bin" 2> /dev/null || continue
        # The 12 bytes that turn 32 raw Ed25519 key bytes into a DER public key.
        { printf '\060\052\060\005\006\003\053\145\160\003\041\000'; printf '%s' "$b64" | openssl base64 -d -A; } > "$TMP/key.der" 2> /dev/null || continue
        openssl pkey -pubin -inform DER -in "$TMP/key.der" -out "$TMP/key.pem" 2> /dev/null || continue
        if openssl pkeyutl -verify -pubin -inkey "$TMP/key.pem" -rawin -in "$1" -sigfile "$TMP/sig.bin" > /dev/null 2>&1; then return 0; fi
        return 1
    done
    return 1
}

# Pull `"key":"value"` or `"key":number` out of a JSON text with no spaces or newlines in it.
json_field() { printf '%s' "$1" | sed -n 's/.*"'"$2"'":"\{0,1\}\([^",}]*\)"\{0,1\}.*/\1/p' | head -n 1; }

# download_verified <file name> <destination>: download a release file and check it against the signed manifest.
download_verified() {
    dv_name="$1"
    dv_dest="$2"
    mktmp
    dv_base="${AGENTMUX_INSTALL_RELEASE_BASE:-$RELEASES/download/v$VERSION}"
    fetch "$dv_base/AgentMux-$VERSION-manifest.json" "$TMP/manifest.json"
    fetch "$dv_base/AgentMux-$VERSION-envelope.json" "$TMP/envelope.json"
    fetch "$dv_base/AgentMux-$VERSION-envelope.json.sig" "$TMP/envelope.sig"

    dv_sig=0
    verify_signature "$TMP/envelope.json" "$TMP/envelope.sig" || dv_sig=$?
    case "$dv_sig" in
        0) say "signature of the release checked" ;;
        1) die "the release's signature does not match: not installing" ;;
        *)
            [ "$REQUIRE_SIGNATURE" = 0 ] || die "the release's signature cannot be checked here (it needs OpenSSL 3) and --require-signature was given"
            say "signature not checked: this system's openssl cannot (it needs OpenSSL 3); checking the hashes only"
            ;;
    esac

    dv_env="$(tr -d ' \n\r\t' < "$TMP/envelope.json")"
    [ "$(json_field "$dv_env" tag)" = "v$VERSION" ] || die "the envelope is for another release"
    dv_want_manifest="$(json_field "$dv_env" manifest_sha256)"
    [ -n "$dv_want_manifest" ] && [ "$(sha256_of "$TMP/manifest.json")" = "$dv_want_manifest" ] || die "the manifest is not the one the envelope names"

    dv_man="$(tr -d ' \n\r\t' < "$TMP/manifest.json")"
    dv_entry="$(printf '%s' "$dv_man" | grep -o '{"name":"'"$(printf '%s' "$dv_name" | sed 's/\./\\./g')"'","size":[0-9]*,"sha256":"[0-9a-f]*"}' | head -n 1 || true)"
    [ -n "$dv_entry" ] || die "the signed manifest does not list $dv_name"
    dv_want_size="$(json_field "$dv_entry" size)"
    dv_want_sha="$(json_field "$dv_entry" sha256)"

    printf '+ download %s\n' "$dv_name"
    fetch "$dv_base/$dv_name" "$dv_dest"
    dv_got_size="$(wc -c < "$dv_dest" | tr -d ' ')"
    [ "$dv_got_size" = "$dv_want_size" ] || { rm -f "$dv_dest"; die "$dv_name is $dv_got_size bytes, the manifest says $dv_want_size"; }
    [ "$(sha256_of "$dv_dest")" = "$dv_want_sha" ] || { rm -f "$dv_dest"; die "$dv_name does not match the manifest's SHA-256"; }
    say "$dv_name matches the signed manifest"
}

# ---- Linux ----
linux_pm() {
    if [ -n "${AGENTMUX_INSTALL_PM:-}" ]; then
        if [ "$AGENTMUX_INSTALL_PM" != none ]; then echo "$AGENTMUX_INSTALL_PM"; fi # "none" is for the tests
        return
    fi
    if have apt-get; then echo apt
    elif have dnf; then echo dnf
    elif have zypper; then echo zypper
    fi
}

install_linux_repo() {
    pm="$1"
    say "adding the signed AgentMux repository and installing with $pm (this needs root)"
    need_root_tool
    case "$pm" in
        apt)
            fetch_to_system "$PKG_HOST/agentmux-archive-keyring.gpg" /usr/share/keyrings/agentmux-archive-keyring.gpg
            fetch_to_system "$PKG_HOST/agentmux.sources" /etc/apt/sources.list.d/agentmux.sources
            run $SUDO apt-get update
            run $SUDO apt-get install -y agentmux
            ;;
        dnf)
            fetch_to_system "$PKG_HOST/agentmux.repo" /etc/yum.repos.d/agentmux.repo
            run $SUDO dnf install -y agentmux
            ;;
        zypper)
            if [ "$DRY_RUN" = 1 ] || ! $SUDO zypper lr agentmux > /dev/null 2>&1; then
                run $SUDO zypper --non-interactive addrepo --refresh "$PKG_HOST/agentmux.repo"
            fi
            run $SUDO zypper --non-interactive --gpg-auto-import-keys refresh
            run $SUDO zypper --non-interactive install -y agentmux
            ;;
        *) die "unknown package manager: $pm" ;;
    esac
}

install_linux_portable() {
    have tar || die "tar is needed"
    resolve_version
    name="AgentMux_${VERSION}_amd64-portable.tar.xz"
    dest="$HOME/.local/share/agentmux"
    say "no package manager found (or --method download): installing the portable copy of $VERSION into $dest"
    if [ "$DRY_RUN" = 1 ]; then
        printf '+ download %s and check it against the signed manifest\n' "$name"
        printf '+ tar -xJf %s -C %s\n' "$name" "$dest"
        printf '+ ln -sf %s/AgentMux/agentmux.sh %s/.local/bin/agentmux\n' "$dest" "$HOME"
        return
    fi
    mktmp
    download_verified "$name" "$TMP/$name"
    mkdir -p "$dest" "$HOME/.local/bin"
    rm -rf "$dest/AgentMux"
    tar -xJf "$TMP/$name" -C "$dest" || die "could not unpack $name (is xz installed?)"
    [ -x "$dest/AgentMux/agentmux.sh" ] || die "the tarball has no AgentMux/agentmux.sh"
    ln -sf "$dest/AgentMux/agentmux.sh" "$HOME/.local/bin/agentmux"
    case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) say "add $HOME/.local/bin to your PATH to run  agentmux" ;; esac
}

launch_linux() {
    [ "$NO_LAUNCH" = 0 ] && [ "$DRY_RUN" = 0 ] || return 0
    if [ -z "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then say "installed. No display here: run  agentmux  from a desktop session."; return 0; fi
    agentmux_bin="$(command -v agentmux || echo "$HOME/.local/bin/agentmux")"
    say "starting AgentMux"
    nohup "$agentmux_bin" > /dev/null 2>&1 < /dev/null &
}

# ---- macOS ----
tap_exists() {
    have curl && curl -fsI --proto '=https' -o /dev/null "$TAP_CASK_URL" 2> /dev/null
}

install_macos_brew() {
    run brew install --cask agentmuxai/tap/agentmux
}

install_macos_dmg() {
    resolve_version
    name="AgentMux_${VERSION}_arm64.dmg"
    if [ -w /Applications ]; then target=/Applications; else target="$HOME/Applications"; fi
    say "installing $VERSION from the .dmg into $target"
    if [ "$DRY_RUN" = 1 ]; then
        printf '+ download %s and check it against the signed manifest\n' "$name"
        printf '+ hdiutil attach, copy AgentMux.app to %s, hdiutil detach\n' "$target"
        return
    fi
    have hdiutil || die "hdiutil is needed"
    mktmp
    download_verified "$name" "$TMP/$name"
    mkdir -p "$target" "$TMP/mnt"
    hdiutil attach -nobrowse -readonly -mountpoint "$TMP/mnt" "$TMP/$name" > /dev/null || die "could not open the .dmg"
    rm -rf "$target/AgentMux.app"
    if ! ditto "$TMP/mnt/AgentMux.app" "$target/AgentMux.app"; then hdiutil detach "$TMP/mnt" > /dev/null 2>&1 || true; die "could not copy AgentMux.app to $target"; fi
    hdiutil detach "$TMP/mnt" > /dev/null || true
}

launch_macos() {
    [ "$NO_LAUNCH" = 0 ] && [ "$DRY_RUN" = 0 ] || return 0
    say "starting AgentMux"
    open -a AgentMux 2> /dev/null || open "/Applications/AgentMux.app" 2> /dev/null || open "$HOME/Applications/AgentMux.app" || true
}

# ---- the server package ----
# Is the name `agentmux` free for the server's program? The desktop app's launcher has that name, and is not replaced.
agentmux_name_is_free() { # <the link this install owns>
    found="$(command -v agentmux 2> /dev/null || true)"
    [ -z "$found" ] || [ "$(readlink "$found" 2> /dev/null || true)" = "$1" ]
}

install_server() {
    have tar || die "tar is needed"
    resolve_version
    case "$ARCH" in x86_64) sarch=x64 ;; *) sarch="$ARCH" ;; esac
    sname="agentmux-server-${VERSION}-${OS}-${sarch}"
    base="$HOME/.agentmux/server"
    dest="$base/$VERSION"
    target="$base/current/bin/agentmux"
    say "installing the AgentMux server $VERSION into $dest"
    if [ "$DRY_RUN" = 1 ]; then
        printf '+ download %s.tar.gz and check it against the signed manifest\n' "$sname"
        printf '+ tar -xzf %s.tar.gz into %s\n' "$sname" "$dest"
        printf '+ ln -sfn %s %s/current\n' "$dest" "$base"
        printf '+ ln -sf %s %s/.local/bin/agentmux-server\n' "$target" "$HOME"
        if agentmux_name_is_free "$HOME/.local/bin/agentmux"; then printf '+ ln -sf %s %s/.local/bin/agentmux\n' "$target" "$HOME"; fi
        if [ "$SETUP" = 1 ]; then printf '+ agentmux-server setup --yes --no-open\n'; fi
        return
    fi
    mktmp
    download_verified "$sname.tar.gz" "$TMP/$sname.tar.gz"
    mkdir -p "$base" "$HOME/.local/bin"
    rm -rf "$dest.new"
    mkdir -p "$dest.new"
    tar -xzf "$TMP/$sname.tar.gz" -C "$dest.new" --strip-components=1 || { rm -rf "$dest.new"; die "could not unpack $sname.tar.gz"; }
    if [ ! -x "$dest.new/bin/agentmux" ] || [ ! -x "$dest.new/bin/agentmux-srv" ] || [ ! -f "$dest.new/web/index.html" ]; then
        rm -rf "$dest.new"
        die "$sname.tar.gz is not an AgentMux server package (no bin/agentmux, bin/agentmux-srv or web/index.html)"
    fi
    # The server of the version being replaced keeps running from its own folder until it is restarted.
    rm -rf "$dest"
    mv "$dest.new" "$dest"
    ln -sfn "$dest" "$base/current"
    # `agentmux-server` is always this program's name on the PATH; `agentmux` too when nothing else has it.
    cmd=agentmux-server
    ln -sf "$target" "$HOME/.local/bin/agentmux-server"
    if agentmux_name_is_free "$HOME/.local/bin/agentmux"; then
        ln -sf "$target" "$HOME/.local/bin/agentmux"
        cmd=agentmux
    else
        say "something else is already named agentmux here (the desktop app?): run this one as  agentmux-server"
    fi
    case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) say "add $HOME/.local/bin to your PATH to run  $cmd" ;; esac
    if [ "$SETUP" = 1 ]; then
        "$HOME/.local/bin/$cmd" setup --yes --no-open || die "setup did not finish: run  $cmd doctor  to see why"
    else
        say "next: run  $cmd setup  to choose a port, start the server and sign in"
    fi
}

# ---- go ----
# Everything above only defines functions; nothing is installed until main runs at the very end,
# so a download cut short (curl | sh) does nothing.
main() {
    if [ "$DRY_RUN" = 1 ]; then mode=", dry run"; else mode=""; fi
    say "agentmux-install $SCRIPT_VERSION: $OS $ARCH$mode"

    if [ "$SERVER" = 1 ]; then
        install_server
        say "done"
        return
    fi

    if [ "$OS" = linux ]; then
        PM="$(linux_pm || true)"
        if [ "$METHOD" != download ] && [ -n "$PM" ]; then
            if [ -n "$VERSION" ]; then say "note: --version is not used with the package repository: it installs the newest version it has. Use --method download for $VERSION."; fi
            install_linux_repo "$PM"
        else
            [ "$METHOD" != package-manager ] || die "no supported package manager (apt, dnf or zypper) found"
            install_linux_portable
        fi
        launch_linux
    else
        if [ "$METHOD" != download ] && have brew && tap_exists; then
            install_macos_brew
        else
            [ "$METHOD" != package-manager ] || die "Homebrew or its AgentMux cask is not available"
            install_macos_dmg
        fi
        launch_macos
    fi
    say "done"
}

main

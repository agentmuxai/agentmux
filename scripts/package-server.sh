#!/usr/bin/env bash
# Assemble the AgentMux Headless package: the programs and the built web UI, as one tarball per platform.
#
#   scripts/package-server.sh --os <linux|macos|windows> --arch <x64|arm64> --bin-dir <dir> --web-dir <dir> --out <dir> [--version X.Y.Z]
#
# --bin-dir holds the four built programs (agentmux, agentmux-srv, agentmux-mcp, agentmux-bashwrap; .exe on Windows), e.g. target/release;
# --web-dir is the web UI build (node scripts/build-web.mjs, dist/web). It writes
#   <out>/agentmux-headless-<version>-<os>-<arch>.tar.gz (on Windows, x64 only: a .zip, scripts/zip-deterministic.mjs)
# whose one top folder holds bin/, web/ and VERSION. scripts/install/install.sh --headless installs it, after checking it
# against the release's signed manifest. Design: docs/specs/SPEC_HEADLESS_SERVER_PACKAGE_AND_LOCAL_WEB_UI_2026_11_12.md §4.
set -euo pipefail

usage() { sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; }
die() { echo "package-server: $*" >&2; exit 1; }

OS="" ARCH="" BIN_DIR="" WEB_DIR="" OUT="" VERSION=""
while [ $# -gt 0 ]; do
    case "$1" in
        --os) OS="${2:?--os needs a value}"; shift 2 ;;
        --arch) ARCH="${2:?--arch needs a value}"; shift 2 ;;
        --bin-dir) BIN_DIR="${2:?--bin-dir needs a value}"; shift 2 ;;
        --web-dir) WEB_DIR="${2:?--web-dir needs a value}"; shift 2 ;;
        --out) OUT="${2:?--out needs a value}"; shift 2 ;;
        --version) VERSION="${2:?--version needs a value}"; shift 2 ;;
        --help | -h) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

case "$OS" in linux | macos | windows) ;; *) die "--os is linux, macos or windows" ;; esac
case "$ARCH" in x64 | arm64) ;; *) die "--arch is x64 or arm64" ;; esac
# Windows is x64 only (docs/specs/SPEC_HEADLESS_RENAME_AND_PER_PLATFORM_2026_11_16.md 5.1, 8): a .zip of .exe files.
[ "$OS" != windows ] || [ "$ARCH" = x64 ] || die "Windows is x64 only"
EXE=""
[ "$OS" != windows ] || EXE=".exe"
[ -n "$BIN_DIR" ] && [ -d "$BIN_DIR" ] || die "--bin-dir must be a folder with the built programs"
[ -n "$WEB_DIR" ] && [ -f "$WEB_DIR/index.html" ] || die "--web-dir must be the web UI build (a folder with index.html)"
[ -n "$OUT" ] || die "--out is needed"

if [ -z "$VERSION" ]; then
    # The workspace version, which is the desktop's version.
    ROOT="$(cd "$(dirname "$0")/.." && pwd)"
    VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
fi
printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.]+)?$' || die "the version '$VERSION' does not look like 0.60.14"

TOP="agentmux-headless-${VERSION}-${OS}-${ARCH}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/$TOP/bin" "$OUT"

for prog in agentmux agentmux-srv agentmux-mcp agentmux-bashwrap; do
    [ -f "$BIN_DIR/$prog$EXE" ] || die "$BIN_DIR/$prog$EXE is not built"
    install -m 0755 "$BIN_DIR/$prog$EXE" "$STAGE/$TOP/bin/$prog$EXE"
done
# Windows only: agentmuxw, the no-console stub the sign-in task runs (a GUI-subsystem program that starts `agentmux run`
# hidden). Packed when it was built; the workflow that makes a release says whether it must be there.
if [ "$OS" = windows ] && [ -f "$BIN_DIR/agentmuxw.exe" ]; then
    install -m 0755 "$BIN_DIR/agentmuxw.exe" "$STAGE/$TOP/bin/agentmuxw.exe"
fi
cp -R "$WEB_DIR" "$STAGE/$TOP/web"
printf '%s\n' "$VERSION" > "$STAGE/$TOP/VERSION"

# Fixed owner, fixed order and a fixed time, so the same inputs make the same bytes.
if [ "$OS" = windows ]; then
    # A .zip, written by scripts/zip-deterministic.mjs (the runners have no `zip` and Compress-Archive keeps no fixed order or time).
    pack="$STAGE/$TOP"
    archive="$OUT/$TOP.zip"
    if command -v cygpath > /dev/null 2>&1; then pack="$(cygpath -m "$pack")"; archive="$(cygpath -m "$archive")"; fi
    node "$(dirname "$0")/zip-deterministic.mjs" "$pack" "$archive" > /dev/null
    archive="$OUT/$TOP.zip"
elif tar --version 2> /dev/null | grep -q 'GNU tar'; then
    archive="$OUT/$TOP.tar.gz"
    (cd "$STAGE" && tar --sort=name --owner=0 --group=0 --numeric-owner --mtime='2026-01-01 00:00:00Z' -cf - "$TOP") | gzip -n -9 > "$archive"
else
    archive="$OUT/$TOP.tar.gz"
    (cd "$STAGE" && tar -cf - "$TOP") | gzip -n -9 > "$archive"
fi

if command -v sha256sum > /dev/null 2>&1; then
    sum="$(sha256sum < "$archive" | cut -d ' ' -f 1)"
else
    sum="$(shasum -a 256 < "$archive" | cut -d ' ' -f 1)"
fi
echo "package-server: $archive ($(wc -c < "$archive" | tr -d ' ') bytes, sha256 $sum)"

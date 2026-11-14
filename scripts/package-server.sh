#!/usr/bin/env bash
# Assemble the AgentMux server package: the programs and the built web UI, as one tarball per platform.
#
#   scripts/package-server.sh --os <linux|macos> --arch <x64|arm64> --bin-dir <dir> --web-dir <dir> --out <dir> [--version X.Y.Z]
#
# --bin-dir holds the four built programs (agentmux, agentmux-srv, agentmux-mcp, agentmux-bashwrap), e.g. target/release;
# --web-dir is the web UI build (node scripts/build-web.mjs, dist/web). It writes
#   <out>/agentmux-server-<version>-<os>-<arch>.tar.gz
# whose one top folder holds bin/, web/ and VERSION. scripts/install/install.sh --server installs it, after checking it
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

case "$OS" in linux | macos) ;; *) die "--os is linux or macos" ;; esac
case "$ARCH" in x64 | arm64) ;; *) die "--arch is x64 or arm64" ;; esac
[ -n "$BIN_DIR" ] && [ -d "$BIN_DIR" ] || die "--bin-dir must be a folder with the built programs"
[ -n "$WEB_DIR" ] && [ -f "$WEB_DIR/index.html" ] || die "--web-dir must be the web UI build (a folder with index.html)"
[ -n "$OUT" ] || die "--out is needed"

if [ -z "$VERSION" ]; then
    # The workspace version, which is the desktop's version.
    ROOT="$(cd "$(dirname "$0")/.." && pwd)"
    VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
fi
printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.]+)?$' || die "the version '$VERSION' does not look like 0.60.14"

TOP="agentmux-server-${VERSION}-${OS}-${ARCH}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/$TOP/bin" "$OUT"

for prog in agentmux agentmux-srv agentmux-mcp agentmux-bashwrap; do
    [ -f "$BIN_DIR/$prog" ] || die "$BIN_DIR/$prog is not built"
    install -m 0755 "$BIN_DIR/$prog" "$STAGE/$TOP/bin/$prog"
done
cp -R "$WEB_DIR" "$STAGE/$TOP/web"
printf '%s\n' "$VERSION" > "$STAGE/$TOP/VERSION"

# Fixed owner, fixed order and a fixed time, so the same inputs make the same bytes.
tarball="$OUT/$TOP.tar.gz"
if tar --version 2> /dev/null | grep -q 'GNU tar'; then
    (cd "$STAGE" && tar --sort=name --owner=0 --group=0 --numeric-owner --mtime='2026-01-01 00:00:00Z' -cf - "$TOP") | gzip -n -9 > "$tarball"
else
    (cd "$STAGE" && tar -cf - "$TOP") | gzip -n -9 > "$tarball"
fi

if command -v sha256sum > /dev/null 2>&1; then sum="$(sha256sum < "$tarball" | cut -d ' ' -f 1)"; else sum="$(shasum -a 256 < "$tarball" | cut -d ' ' -f 1)"; fi
echo "package-server: $tarball ($(wc -c < "$tarball" | tr -d ' ') bytes, sha256 $sum)"

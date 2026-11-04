#!/usr/bin/env bash
# The runtime dependencies of the .deb and the .rpm, from one list.
#
# Usage:
#   bash scripts/linux-package-deps.sh deb <root>     one Depends entry per line
#   bash scripts/linux-package-deps.sh rpm <root>     one Requires entry per line
#   bash scripts/linux-package-deps.sh check <root>   fail when the staged tree under <root>
#                                                     needs a library nobody bundles or lists
#
# The list is scripts/linux-package-deps.list (read it for the format and the reasoning).
# `check` reads the NEEDED entries of every ELF file under <root> with readelf, drops what the
# tree carries itself (a file of that name, or a library with that SONAME) and what glibc and
# libgcc bring, and compares the rest with the list. A library the runtime now needs but the
# list lacks fails the build, which is the only way the package would otherwise find out: a
# minimal install without it cannot start. A listed library nothing needs any more is only
# reported, since over-declaring does not break an install.
#
# `deb` and `rpm` also print the glibc floor of the staged tree under <root>: the highest
# GLIBC_x.y version any of its ELF files links against, read from the binaries themselves
# (`libc6 (>= 2.43)`, `libc.so.6(GLIBC_2.43)(64bit)`), so it follows the runtime up and down.
# A system with an older glibc then refuses the install with a reason instead of installing
# an app that cannot start.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIST="${LINUX_PACKAGE_DEPS_LIST:-$HERE/linux-package-deps.list}"

# Every system that runs a .deb or .rpm has these; they are never declared.
BASE_SONAMES='ld-linux-x86-64.so.2 libc.so.6 libm.so.6 libgcc_s.so.1 libdl.so.2 libpthread.so.0 librt.so.1'

# "<soname>|<alt>|<alt>" per list line, comments and blanks dropped, spaces trimmed.
entries() {
    sed -e 's/#.*//' -e 's/[[:space:]]*|[[:space:]]*/|/g' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' "$LIST" | grep -v '^$' || true
}

# The highest GLIBC_x.y version any ELF file under $1 requires (empty when none does).
glibc_floor() {
    local root="$1" f
    command -v readelf >/dev/null || { echo "linux-package-deps: readelf not found (install binutils)" >&2; exit 2; }
    while IFS= read -r -d '' f; do
        [ "$(head -c4 "$f" 2>/dev/null | tr -d '\0')" = $'\x7f'ELF ] || continue
        readelf -V "$f" 2>/dev/null | sed -n 's/.*Name: GLIBC_\([0-9][0-9.]*\).*/\1/p'
    done < <(find "$root" -type f -print0) | sort -uV | tail -1
}

case "${1:-}" in
    deb)
        root="${2:?usage: linux-package-deps.sh deb <root>}"
        # One Depends entry per distinct package set: "libasound2t64 | libasound2".
        entries | cut -d'|' -f2- | sed 's/|/ | /g' | sort -u
        floor="$(glibc_floor "$root")"
        [ -z "$floor" ] || echo "libc6 (>= $floor)"
        ;;
    rpm)
        root="${2:?usage: linux-package-deps.sh rpm <root>}"
        # The soname itself: Fedora and openSUSE resolve it to their own package.
        entries | cut -d'|' -f1 | sort -u | sed 's/$/()(64bit)/'
        floor="$(glibc_floor "$root")"
        [ -z "$floor" ] || echo "libc.so.6(GLIBC_$floor)(64bit)"
        ;;
    check)
        root="${2:?usage: linux-package-deps.sh check <root>}"
        [ -d "$root" ] || { echo "linux-package-deps: $root is not a directory" >&2; exit 2; }
        command -v readelf >/dev/null || { echo "linux-package-deps: readelf not found (install binutils)" >&2; exit 2; }
        needed="$(mktemp)"; have="$(mktemp)"; trap 'rm -f "$needed" "$have"' EXIT
        # NEEDED sonames, and what the tree provides (file names and SONAMEs), of every ELF file.
        while IFS= read -r -d '' f; do
            [ "$(head -c4 "$f" 2>/dev/null | tr -d '\0')" = $'\x7f'ELF ] || continue
            readelf -d "$f" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' >> "$needed"
            readelf -d "$f" 2>/dev/null | sed -n 's/.*(SONAME).*\[\(.*\)\]/\1/p' >> "$have"
            basename "$f" >> "$have"
        done < <(find "$root" -type f -print0)
        # Symlinked library names (libfoo.so.1 -> libfoo.so.1.2.3) count as provided too.
        find "$root" -type l -printf '%f\n' >> "$have"
        printf '%s\n' $BASE_SONAMES >> "$have"
        listed="$(entries | cut -d'|' -f1 | sort -u)"
        missing="$(sort -u "$needed" | grep -vxF -f <(sort -u "$have") | grep -vxF -f <(printf '%s\n' "$listed") || true)"
        if [ -n "$missing" ]; then
            echo "linux-package-deps: the runtime under $root needs system libraries the package neither bundles nor declares:" >&2
            printf '  %s\n' $missing >&2
            echo "Add each to scripts/linux-package-deps.list with its Debian package, or bundle it." >&2
            exit 1
        fi
        unused="$(printf '%s\n' "$listed" | grep -vxF -f <(sort -u "$needed") || true)"
        if [ -n "$unused" ]; then
            echo "linux-package-deps: listed but no longer needed (still declared, harmless):" >&2
            printf '  %s\n' $unused >&2
        fi
        echo "linux-package-deps: ok ($(printf '%s\n' "$listed" | wc -l | tr -d ' ') declared libraries cover $(sort -u "$needed" | wc -l | tr -d ' ') needed)"
        ;;
    *)
        echo "usage: linux-package-deps.sh deb <root> | rpm <root> | check <root>" >&2
        exit 2
        ;;
esac

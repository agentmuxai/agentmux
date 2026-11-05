#!/usr/bin/env bash
# Verify that no shared library or executable in a Linux CEF runtime directory needs a newer
# glibc than the floor AgentMux supports.
#
# A CEF built with use_sysroot=false links against the build host's libraries, so the runtime
# needs the build host's glibc: the 0.60.14 libcef.so, libGLESv2.so and libvk_swiftshader.so
# needed GLIBC_2.43 (built on Ubuntu 26.04) and the app would not start on Ubuntu 24.04
# (2.39), Debian 13 or Fedora 42. A build against Chromium's Debian sysroot (use_sysroot=true,
# glibc 2.31) passes. The host binaries are built on Ubuntu 22.04 and need 2.34, so that is
# the default floor.
#
# Usage: verify-glibc-floor.sh <dir> [max-glibc-version, default 2.34]
# Looks at every ELF file under <dir> (the .so files, chrome-sandbox and the like).
# Exit 0 if every one needs at most that GLIBC_ version, or none needs glibc at all.
# Exit 1 naming each offender and the version it needs. Exit 2 on bad usage or no objdump.
set -uo pipefail

dir="${1:?usage: verify-glibc-floor.sh <dir> [max-glibc-version]}"
max="${2:-2.34}"

if [ ! -d "$dir" ]; then
  echo "verify-glibc-floor: $dir is not a directory" >&2
  exit 2
fi
if ! command -v objdump >/dev/null 2>&1; then
  echo "verify-glibc-floor: objdump not found (install binutils)" >&2
  exit 2
fi

checked=0
bad=0
while IFS= read -r -d '' f; do
  # ELF only: the first four bytes are 0x7f 'E' 'L' 'F'.
  [ "$(head -c 4 "$f" 2>/dev/null | od -An -c | tr -d ' ')" = '177ELF' ] || continue
  checked=$((checked + 1))
  need="$(objdump -T "$f" 2>/dev/null | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -Vu | tail -1)"
  [ -n "$need" ] || continue
  top="$(printf '%s\n%s\n' "$need" "$max" | sort -V | tail -1)"
  if [ "$top" != "$max" ]; then
    echo "FAIL  ${f#"$dir"/} needs GLIBC_$need (floor is GLIBC_$max)" >&2
    bad=$((bad + 1))
  fi
done < <(find -L "$dir" -type f -print0)

if [ "$checked" -eq 0 ]; then
  echo "verify-glibc-floor: no ELF files under $dir" >&2
  exit 2
fi
if [ "$bad" -gt 0 ]; then
  echo "verify-glibc-floor: $bad of $checked files need a newer glibc than $max: build with use_sysroot=true" >&2
  exit 1
fi
echo "verify-glibc-floor: $checked ELF files, none needs glibc newer than $max"

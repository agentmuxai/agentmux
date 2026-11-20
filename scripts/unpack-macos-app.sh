#!/usr/bin/env bash
# Unpack the unsigned AgentMux.app that the release's build job hands to its signing job, refusing anything that is
# not an .app laid out inside its own folder.
#
# Usage:  unpack-macos-app.sh <AgentMux-unsigned-app.zip> <destination directory>
#
# The zip is input from code that ran in another job (the built commit's scripts and its dependencies' scripts), so
# it is checked before it is trusted:
#   before unpacking  every entry is AgentMux.app/... (or the resource-fork folder ditto writes, __MACOSX/...), with
#                     no absolute path, no `..` component, no backslash and no empty component;
#   after unpacking   the only thing at the top is AgentMux.app, it has an Info.plist, it holds nothing but regular
#                     files, folders and symlinks, and every symlink's target, followed all the way, stays inside
#                     AgentMux.app (an absolute target is refused: the app uses relative links only).
# It starts nothing from the archive: it lists it, unpacks it and looks at the result.
#
# Exit: 0 unpacked and checked (the app is <destination>/AgentMux.app); 1 anything else, with nothing left unpacked.
set -euo pipefail

ZIP="${1:-}"
DEST="${2:-}"
[ -f "$ZIP" ] || { echo "❌ unpack: not a file: ${ZIP:-<none>}" >&2; exit 1; }
[ -n "$DEST" ] || { echo "usage: unpack-macos-app.sh <zip> <destination>" >&2; exit 1; }

fail() { echo "❌ unpack: $1" >&2; rm -rf "$DEST/AgentMux.app" "$DEST/__MACOSX"; exit 1; }

# 1. The entries, before anything is written. zipinfo -1 prints one name per line; a name with a line break in it
# comes out as two lines that cannot both pass the pattern below.
entries="$(zipinfo -1 "$ZIP")" || { echo "❌ unpack: not a readable zip" >&2; exit 1; }
[ -n "$entries" ] || { echo "❌ unpack: the zip is empty" >&2; exit 1; }
while IFS= read -r name; do
    case "$name" in
        AgentMux.app|AgentMux.app/*|__MACOSX|__MACOSX/*) ;;
        *) echo "❌ unpack: an entry is outside AgentMux.app: '$name'" >&2; exit 1 ;;
    esac
    case "$name" in
        *\\*) echo "❌ unpack: an entry has a backslash: '$name'" >&2; exit 1 ;;
        */../*|*/..|../*|..|*//*|/*) echo "❌ unpack: an entry leaves its folder: '$name'" >&2; exit 1 ;;
    esac
done <<< "$entries"

# 2. Unpack, then look at what is there.
mkdir -p "$DEST"
[ -z "$(ls -A "$DEST" 2> /dev/null)" ] || { echo "❌ unpack: the destination is not empty: $DEST" >&2; exit 1; }
ditto -x -k "$ZIP" "$DEST"

top="$(ls -A "$DEST")"
[ "$top" = "AgentMux.app" ] || fail "the top of the archive is not AgentMux.app alone (found: $(printf '%s' "$top" | tr '\n' ' '))"
APP="$DEST/AgentMux.app"
[ -d "$APP/Contents" ] && [ -f "$APP/Contents/Info.plist" ] || fail "AgentMux.app has no Contents/Info.plist"
if [ -n "$(find "$APP" ! -type f ! -type d ! -type l -print -quit)" ]; then
    fail "AgentMux.app holds a file that is not a regular file, a folder or a symlink"
fi

# 3. Every symlink stays inside the app. realpath follows the whole chain, so a link into a link that leaves is caught.
python3 -I - "$APP" <<'PY' || fail "a symlink leaves AgentMux.app"
import os, sys
app = os.path.realpath(sys.argv[1])
bad = []
for root, dirs, files in os.walk(sys.argv[1]):
    for name in dirs + files:
        path = os.path.join(root, name)
        if not os.path.islink(path):
            continue
        target = os.readlink(path)
        if os.path.isabs(target):
            bad.append((path, target))
            continue
        real = os.path.realpath(os.path.join(os.path.dirname(path), target))
        if real != app and not real.startswith(app + os.sep):
            bad.append((path, target))
for path, target in bad:
    print("  symlink %s -> %s" % (os.path.relpath(path, sys.argv[1]), target), file=sys.stderr)
sys.exit(1 if bad else 0)
PY
echo "✓ unpacked and checked: $APP"

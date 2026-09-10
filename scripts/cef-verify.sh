#!/usr/bin/env bash
# cef-verify.sh — verify a CEF fork ref carries the whole AgentMux carry-set.
#
# docs/cef-build/CEF_FORK_MAINTENANCE.md (§3 the inventory, §5 this gate)
#
# THE RULE: AgentMux carries four changes on top of CEF, spanning 18 CEF source
# files and 3 Chromium-side patches. Every one must survive every milestone
# upgrade. Layer A (patches under cef/patch/) fails loudly when it goes missing;
# Layer B (edits to libcef/** and include/**) fails SILENTLY — the tree still
# compiles, because our additions are additive. This gate is the only thing that
# checks Layer B.
#
# Why this is a script and not a snippet in the doc. Three separate versions of
# these checks, written as markdown code blocks, shipped bugs that made them
# incapable of failing:
#   * a `sed` without /g redirected only the depfile and left `-o` pointing at
#     the real object — the "verification" OVERWROTE the build output;
#   * a pristine build compiled from /tmp differed from the shipped object on
#     the embedded DWARF source path alone, so the comparison could never fire;
#   * `git -C ""` does not fail — it answers for the current directory, which
#     silently returned Chromium's HEAD instead of the fork's.
# None were visible by reading. All three die to a single test run, which is
# what scripts/cef-verify.test.sh now does on every PR.
#
# Usage:
#   scripts/cef-verify.sh                      # milestone 7778, probe for the clone
#   scripts/cef-verify.sh --ref 7977           # a milestone => <remote>/<ref>
#   scripts/cef-verify.sh --ref a1b2c3d        # an exact ref/tag/SHA, used verbatim
#   scripts/cef-verify.sh --repo ~/src/cef     # explicit clone (validated, never guessed past)
#   scripts/cef-verify.sh --remote fork        # whatever you named agentmuxai/cef
#
# Exit 0 = all 21 present. Exit 1 = something is missing, or the clone/ref could
# not be resolved. It never modifies the repository it inspects.

set -euo pipefail

REPO="" ; REF="7778" ; REMOTE="${REMOTE:-agentmuxai}" ; QUIET=0
while [ $# -gt 0 ]; do
  case "$1" in
    --repo)   REPO="${2:-}"; shift 2 ;;
    --ref)    REF="${2:-}"; shift 2 ;;
    --remote) REMOTE="${2:-}"; shift 2 ;;
    --quiet)  QUIET=1; shift ;;
    -h|--help) sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "cef-verify: unknown argument '$1'" >&2; exit 1 ;;
  esac
done

say() { [ "$QUIET" = 1 ] || printf '%s\n' "$*"; }

# ── Locate the fork clone ───────────────────────────────────────────────────
# A .git-less mirror (rsync --exclude=.git / robocopy /XD .git) is NOT a clone:
# `git -C` on it walks UP and answers for the enclosing Chromium checkout. The
# git-root test is what rejects that; the remote test rejects an unrelated repo.
is_fork_clone() {
  local d="$1"
  [ -n "$d" ] && [ -d "$d" ] || return 1
  [ "$(git -C "$d" rev-parse --show-toplevel 2>/dev/null)" = "$(cd "$d" 2>/dev/null && pwd -P)" ] || return 1
  git -C "$d" remote -v 2>/dev/null | grep -q 'agentmuxai/cef'
}

if [ -n "$REPO" ]; then
  # An explicit --repo is a HARD requirement. Falling back to a probed path when
  # the override is wrong would report a different checkout's HEAD and look fine.
  is_fork_clone "$REPO" || { echo "cef-verify: --repo '$REPO' is not an agentmuxai/cef git root" >&2; exit 1; }
else
  for c in "$HOME/cef-build/chromium/chromium/src/cef" \
           "$HOME/cef-build/chromium/cef" \
           "$HOME/cef-build/chromium_git/cef"; do
    if is_fork_clone "$c"; then REPO="$c"; break; fi
  done
  [ -n "$REPO" ] || { echo "cef-verify: no agentmuxai/cef clone found (a .git-less mirror does not count); pass --repo" >&2; exit 1; }
fi

# ── Resolve the ref ─────────────────────────────────────────────────────────
if printf '%s' "$REF" | grep -qE '^[0-9]+$'; then
  git -C "$REPO" remote get-url "$REMOTE" >/dev/null 2>&1 \
    || { echo "cef-verify: no remote '$REMOTE' in $REPO; pass --remote" >&2; exit 1; }
  git -C "$REPO" fetch "$REMOTE" --prune >/dev/null 2>&1 || true
  BR="$REMOTE/$REF"
else
  BR="$REF"   # tag / SHA / local branch, used verbatim — see §8 P3
fi
git -C "$REPO" rev-parse --verify -q "${BR}^{commit}" >/dev/null \
  || { echo "cef-verify: cannot resolve '$BR' in $REPO" >&2; exit 1; }

# ── The carry-set ───────────────────────────────────────────────────────────
# <path>|<identifier absent upstream, present in the fork>
# Probe identifiers, not filenames: every one of these files exists upstream.
CARRY_SET='
include/views/cef_window.h|BeginWindowDrag
libcef/browser/views/window_impl.h|BeginWindowDrag
libcef/browser/views/window_impl.cc|BeginWindowDrag
libcef/renderer/blink_glue.h|SetBaseBackgroundColorOverrideTransparent
libcef/renderer/blink_glue.cc|SetBaseBackgroundColorOverrideTransparent
libcef/renderer/browser_config.h|background_transparent
libcef/renderer/render_manager.cc|background_transparent
libcef/common/mojom/cef.mojom|background_transparent
libcef/browser/browser_info_manager.cc|background_transparent
libcef/browser/browser_platform_delegate.cc|background_transparent
libcef/browser/browser_platform_delegate_create.cc|is_views_hosted
libcef/browser/browser_host_base.cc|IsWindowless() || is_views_hosted()
libcef/browser/context.cc|is_transparent
libcef/browser/context.h|transparent_state
libcef/browser/views/browser_view_impl.cc|ApplyToCurrentRWHView
libcef/browser/views/browser_view_impl.h|LayerTreeHost
libcef/browser/views/window_view.cc|CalculateRenderPasses
include/internal/cef_types.h|or a frameless window
'
PATCHES='rwhv_background_opaque_check views_caption_rightclick_passthrough agentmux_process_requirement'

ok=0 ; miss=0
for name in $PATCHES; do
  cfg=$(git -C "$REPO" show "${BR}:patch/patch.cfg" 2>/dev/null || true)
  if git -C "$REPO" cat-file -e "${BR}:patch/patches/${name}.patch" 2>/dev/null \
     && [[ "$cfg" == *"'name': '${name}'"* ]]; then
    ok=$((ok+1)); say "OK   patch/$name"
  else
    miss=$((miss+1)); say "MISS patch/$name"
  fi
done

while IFS='|' read -r path ident; do
  [ -n "$path" ] || continue
  # Substring test in bash, NOT `git show | grep -q`. With `set -o pipefail`
  # grep -q exits on the first match, git show takes SIGPIPE, and the pipeline
  # reports FAILURE despite the match -- a race that depends on how early in the
  # file the match sits. It reported a false MISS on cef_types.h (match at line
  # 446) while passing on every smaller file. Caught by the test suite; the
  # markdown version of this check had the same latent bug and got away with it.
  content=$(git -C "$REPO" show "${BR}:${path}" 2>/dev/null || true)
  if [[ "$content" == *"$ident"* ]]; then
    ok=$((ok+1)); say "OK   $path"
  else
    miss=$((miss+1)); say "MISS $path"
  fi
done <<EOF
$(printf '%s' "$CARRY_SET")
EOF

total=$((ok+miss))
say "--- $BR: $ok OK, $miss MISS (expect $total OK, 0 MISS) ---"
[ "$miss" -eq 0 ]

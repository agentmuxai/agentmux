#!/usr/bin/env bash
# cef-verify.test.sh — tests for scripts/cef-verify.sh.
#
# This file is the whole point of scripts/cef-verify.sh existing. The same
# checks previously lived as markdown code blocks in
# docs/cef-build/CEF_FORK_MAINTENANCE.md, and shipped four bugs that made them
# incapable of failing or actively destructive:
#
#   1. `sed` without /g left `-o` pointing at the real object — the verification
#      OVERWROTE the build output it was verifying.
#   2. A pristine build compiled from /tmp differed from the shipped object on
#      the embedded DWARF source path alone, so the comparison never fired.
#   3. `git -C ""` does not fail; it answers for the current directory, quietly
#      returning Chromium's HEAD instead of the fork's.
#   4. `set -o pipefail` + `grep -q`: grep exits on first match, `git show` takes
#      SIGPIPE, and the pipeline reports failure DESPITE the match — a race that
#      only bit files where the match sits late (cef_types.h, line 446).
#
# None were visible by reading. Every one dies to a test run. Case 4 has a
# dedicated regression test below because it was intermittent, which is the
# worst kind to leave uncovered.
#
# Usage: bash scripts/cef-verify.test.sh    (exit 0 = all pass)

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/cef-verify.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0

ok()   { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad()  { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

# Build a synthetic fork clone that satisfies the whole carry-set. The file
# list is read from the script itself so the fixture cannot drift from it; the
# assertions below are about MECHANICS (parsing, refs, guards, exit codes),
# which is where every real bug has been.
make_fixture() {
  local dir="$1" ; mkdir -p "$dir" ; ( cd "$dir"
    git init -q . && git remote add agentmuxai https://github.com/agentmuxai/cef.git
    mkdir -p patch/patches
    : > patch/patch.cfg
    for n in rwhv_background_opaque_check views_caption_rightclick_passthrough agentmux_process_requirement; do
      echo "stub patch $n" > "patch/patches/$n.patch"
      printf "  {\n    'name': '%s',\n  },\n" "$n" >> patch/patch.cfg
    done
    # One file per carry-set row, containing its identifier.
    sed -n "/^CARRY_SET='/,/^'$/p" "$SCRIPT" | grep '|' | while IFS='|' read -r path ident; do
      [ -n "$path" ] || continue
      mkdir -p "$(dirname "$path")"
      # The identifier goes EARLY, with a lot of content AFTER it. That is the
      # shape that triggers bug 4: `grep -q` exits on the FIRST match, so it
      # stops reading while `git show` is still writing, git takes SIGPIPE, and
      # under pipefail the pipeline reports failure despite the match. Real
      # cef_types.h matches at line 446 of a much larger file, which is why it
      # was the only false MISS.
      #
      # Two earlier versions of this fixture did not reproduce it: 600 lines of
      # padding (fits the ~64 KiB pipe buffer), then 4000 lines with the
      # identifier LAST -- where grep reads to EOF and never exits early, so
      # there is no SIGPIPE at all. Both passed against a deliberately broken
      # script. The bug is about WHERE the match sits, not how big the file is.
      { echo "$ident"
        for _ in $(seq 1 4000); do
          echo "// trailing filler so git show is still writing when grep -q exits"
        done; } > "$path"
    done
    git add -A && git -c user.email=t@t -c user.name=t commit -q -m "carry-set complete"
  ) }

FIX="$TMP/fork"; make_fixture "$FIX"
HEAD_SHA="$(git -C "$FIX" rev-parse HEAD)"

# 1. Complete carry-set -> all present, exit 0.
out=$("$SCRIPT" --repo "$FIX" --ref "$HEAD_SHA" 2>&1); rc=$?
if [ $rc -eq 0 ] && printf '%s' "$out" | grep -q '21 OK, 0 MISS'; then
  ok "complete carry-set: 21 OK, exit 0"
else bad "complete carry-set" "rc=$rc last=$(printf '%s' "$out" | tail -1)"; fi

# 2. REGRESSION (bug 4): identifiers sit ~600 lines into each file. A
#    `git show | grep -q` implementation under pipefail reports false MISSes
#    here. Passing case 1 with these fixtures IS the regression test; assert
#    explicitly that no MISS was printed.
if ! printf '%s' "$out" | grep -q '^MISS'; then
  ok "no false MISS when the identifier sits late in a large file (pipefail/SIGPIPE)"
else bad "pipefail regression" "$(printf '%s' "$out" | grep '^MISS' | head -2 | tr '\n' ' ')"; fi

# 3. A missing Layer B file is caught.
BROKEN="$TMP/broken"; cp -R "$FIX" "$BROKEN"
rm -f "$BROKEN/libcef/renderer/blink_glue.cc"
git -C "$BROKEN" -c user.email=t@t -c user.name=t commit -qam "drop a carry-set file"
out=$("$SCRIPT" --repo "$BROKEN" --ref "$(git -C "$BROKEN" rev-parse HEAD)" 2>&1); rc=$?
if [ $rc -ne 0 ] && printf '%s' "$out" | grep -q 'MISS libcef/renderer/blink_glue.cc'; then
  ok "missing Layer B file -> MISS + non-zero exit"
else bad "missing Layer B file" "rc=$rc"; fi

# 4. A patch present on disk but NOT registered in patch.cfg is caught.
UNREG="$TMP/unreg"; cp -R "$FIX" "$UNREG"
grep -v "agentmux_process_requirement" "$UNREG/patch/patch.cfg" > "$UNREG/patch/patch.cfg.new"
mv "$UNREG/patch/patch.cfg.new" "$UNREG/patch/patch.cfg"
git -C "$UNREG" -c user.email=t@t -c user.name=t commit -qam "unregister a patch"
out=$("$SCRIPT" --repo "$UNREG" --ref "$(git -C "$UNREG" rev-parse HEAD)" 2>&1); rc=$?
if [ $rc -ne 0 ] && printf '%s' "$out" | grep -q 'MISS patch/agentmux_process_requirement'; then
  ok "patch file present but unregistered -> MISS"
else bad "unregistered patch" "rc=$rc"; fi

# 5. (bug 3) A .git-less mirror inside another repo must be REJECTED, not
#    silently answered for by the enclosing checkout.
OUTER="$TMP/outer"; mkdir -p "$OUTER/src/cef"
( cd "$OUTER" && git init -q . && git -c user.email=t@t -c user.name=t commit -q --allow-empty -m outer )
out=$("$SCRIPT" --repo "$OUTER/src/cef" --ref 7778 2>&1); rc=$?
if [ $rc -ne 0 ] && printf '%s' "$out" | grep -q 'not an agentmuxai/cef git root'; then
  ok "git-less mirror rejected (no walk-up into the enclosing repo)"
else bad "mirror walk-up" "rc=$rc out=$out"; fi

# 6. An explicit --repo that is wrong must FAIL, never fall back to a probe path.
out=$("$SCRIPT" --repo "$TMP/does-not-exist" --ref 7778 2>&1); rc=$?
if [ $rc -ne 0 ] && printf '%s' "$out" | grep -q 'not an agentmuxai/cef git root'; then
  ok "invalid explicit --repo is fatal, not a fallback"
else bad "explicit repo override" "rc=$rc"; fi

# 7. A milestone with no such remote fails clearly rather than guessing.
out=$("$SCRIPT" --repo "$FIX" --ref 7778 --remote nosuchremote 2>&1); rc=$?
if [ $rc -ne 0 ] && printf '%s' "$out" | grep -q "no remote 'nosuchremote'"; then
  ok "unknown remote fails with a clear message"
else bad "unknown remote" "rc=$rc"; fi

# 8. An unresolvable ref fails rather than reporting a clean bill of health.
out=$("$SCRIPT" --repo "$FIX" --ref deadbeefdeadbeef 2>&1); rc=$?
if [ $rc -ne 0 ] && printf '%s' "$out" | grep -q 'cannot resolve'; then
  ok "unresolvable ref fails"
else bad "unresolvable ref" "rc=$rc"; fi

# 9. The gate must never modify the repository it inspects.
before=$(git -C "$FIX" status --porcelain; git -C "$FIX" rev-parse HEAD)
"$SCRIPT" --repo "$FIX" --ref "$HEAD_SHA" >/dev/null 2>&1
after=$(git -C "$FIX" status --porcelain; git -C "$FIX" rev-parse HEAD)
if [ "$before" = "$after" ]; then ok "inspected repo is left untouched"
else bad "repo mutated by the gate"; fi

# 10. A FAILED FETCH must be fatal, even when a stale remote-tracking ref still
#     resolves locally. Verifying against yesterday's ref is the exact
#     false-positive that put a wrong claim about the 152 port into the doc
#     (CEF_FORK_MAINTENANCE.md §1.2) -- a confident answer about state that has
#     since moved. Every other test here passes --ref <SHA>, which skips the
#     fetch entirely, so without this case that path is unexercised.
STALE="$TMP/stale"; cp -R "$FIX" "$STALE"
# The URL must still LOOK like the fork (is_fork_clone greps for agentmuxai/cef)
# while being unreachable -- otherwise the repo check rejects it first and this
# test passes without ever reaching the fetch, which is how it was written the
# first time.
git -C "$STALE" remote set-url agentmuxai "$TMP/unreachable/agentmuxai/cef.git"
git -C "$STALE" update-ref refs/remotes/agentmuxai/7778 "$(git -C "$STALE" rev-parse HEAD)"
out=$("$SCRIPT" --repo "$STALE" --ref 7778 2>&1); rc=$?
if [ $rc -ne 0 ] && ! printf '%s' "$out" | grep -q '21 OK, 0 MISS'; then
  ok "failed fetch is fatal, even with a resolvable stale ref"
else bad "stale-ref after failed fetch" "rc=$rc last=$(printf '%s' "$out" | tail -1)"; fi

# ── scripts/cef-verify-patches.sh ───────────────────────────────────────────
# Fake ninja + fake compiler, so the COMMAND-REWRITING logic is covered. That is
# where the destructive bug lived: a non-global replace left `-o` on the real
# object and the "verification" overwrote the build output. The fake compiler
# embeds the SOURCE PATH in its output, modelling the DWARF path embedding that
# made the original comparison unfalsifiable.
PSCRIPT="$HERE/cef-verify-patches.sh"
PT="$TMP/pbuild"; mkdir -p "$PT/out/build/obj" "$PT/bin"
( cd "$PT" && git init -q . )
cat > "$PT/bin/fakecc" <<'CC'
#!/usr/bin/env bash
# Writes "<source-path>|<source-contents>" to the -o target: sensitive to BOTH
# the path (like DWARF) and the content (like real codegen).
out=""; src=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -MF) shift 2 ;;
    *.cc) src="$1"; shift ;;
    *) shift ;;
  esac
done
{ printf '%s|' "$src"; cat "$src"; } > "$out"
CC
cat > "$PT/bin/ninja" <<CC
#!/usr/bin/env bash
# Emits a Chromium-shaped compile line: the object path appears TWICE.
echo "fakecc -MMD -MF obj/x.o.d -c ../../src.cc -o obj/x.o"
CC
chmod +x "$PT/bin/fakecc" "$PT/bin/ninja"
export PATH="$PT/bin:$PATH"

# Committed (pristine) source, then a working-tree modification = "the patch".
echo "pristine body" > "$PT/src.cc"
git -C "$PT" add -A && git -C "$PT" -c user.email=t@t -c user.name=t commit -qm pristine
echo "patched body" > "$PT/src.cc"
( cd "$PT/out/build" && fakecc -MF obj/x.o.d -c ../../src.cc -o obj/x.o )   # the "shipped" object
OBJ_BEFORE=$(shasum < "$PT/out/build/obj/x.o")

out=$("$PSCRIPT" --build-dir "$PT/out/build" --pair "src.cc:obj/x.o" 2>&1); rc=$?
if [ $rc -eq 0 ] && printf '%s' "$out" | grep -q '^OK   src.cc'; then
  ok "patches: shipped==tree and patch changes codegen -> OK"
else bad "patches happy path" "rc=$rc out=$(printf '%s' "$out" | tail -2 | tr '\n' ' ')"; fi

if [ "$OBJ_BEFORE" = "$(shasum < "$PT/out/build/obj/x.o")" ]; then
  ok "patches: the real object is byte-untouched (no -o leaking onto it)"
else bad "patches: REAL OBJECT WAS OVERWRITTEN"; fi

# A "patch" that changes nothing must be reported as a no-op, not as verified.
git -C "$PT" -c user.email=t@t -c user.name=t commit -qam "adopt the patched body upstream"
( cd "$PT/out/build" && fakecc -MF obj/x.o.d -c ../../src.cc -o obj/x.o )
out=$("$PSCRIPT" --build-dir "$PT/out/build" --pair "src.cc:obj/x.o" 2>&1); rc=$?
if [ $rc -ne 0 ] && printf '%s' "$out" | grep -q 'patch is a no-op'; then
  ok "patches: a no-op patch is reported, not silently verified"
else bad "patches no-op detection" "rc=$rc"; fi

printf '\ncef-verify.test: %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]

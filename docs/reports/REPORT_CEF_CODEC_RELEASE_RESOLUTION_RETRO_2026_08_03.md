# Retro: CEF codec-release CI resolution bug — already fixed, verified live

**Author:** AgentO
**Date:** 2026-08-03
**Related:** `docs/specs/STATUS_CEF_PROPRIETARY_CODECS_MACOS_2026_07_27.md`,
`docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md`,
PR [#2353](https://github.com/agentmuxai/agentmux/pull/2353)

## Summary

The macOS codec-rebuild status doc (2026-07-28) flagged that CI's
`agentmuxai/cef` release resolution was picking a stale tag
(`cef-macos-arm64-148.23.21`) instead of the newly-cut
`cef-macos-arm64-148.23.23-codecs`, because `gh release list`'s default order
is not reliably publish-time-descending on that fork — releases cut without
an explicit `--target` inherit a frozen default-branch-HEAD `created_at`
instead of the actual cut time. The doc marked this "not yet remediated" and
called it a destructive-repo decision needing sign-off (retiring the stale
tag).

**By the time this retro was written, it was already fixed** — PR #2353,
*"fix(ci): sort agentmuxai/cef release resolution by publishedAt, not list
order"*, merged 2026-07-28 23:57 PDT, about 30 minutes after the codec
release was published (23:26 PDT) and evidently in direct response to the
same status doc. No further code change was needed this session; the work
here was pulling latest, confirming the fix is real and live, and closing the
loop with hard evidence instead of leaving it as an open question.

## What the fix actually does

All four call sites (`build-macos.yml`, `build-linux.yml`,
`build-windows.yml`, `ci-nightly-artifacts.yml` ×3 for macOS/Linux/Windows)
resolve the CEF tag the same way — explicit sort, no reliance on API default
order:

```bash
TAG=$(gh release list --repo agentmuxai/cef --limit 30 \
        --json tagName,publishedAt \
        --jq '[.[] | select(.tagName | startswith("cef-macos-arm64-"))] | sort_by(.publishedAt) | reverse | .[0].tagName')
```

This sidesteps the `created_at` staleness entirely — `publishedAt` is
reliable regardless of how the release was cut — and needed no destructive
action against the shared `agentmuxai/cef` distribution repo (the stale
`148.23.21` tag is still there, untouched, just no longer selected).

## Verification (this session)

1. Pulled `agentmux` to latest `main` (`f0ab628b1`) and confirmed PR #2353 is
   included.
2. Re-ran the exact resolver query against live `agentmuxai/cef` data for all
   three platforms:
   - macOS: `cef-macos-arm64-148.23.23-codecs` ✓ (vs. stale `.21` under the
     old unsorted query)
   - Linux: `cef-linux-x86_64-148.0.7778.180-codecs` ✓ (only one
     `-codecs`-suffixed tag, sorts correctly ahead of three older ones)
   - Windows: `cef-windows-x86_64-148.0.7778.180` ✓ (only one tag exists —
     no ambiguity, but confirmed present)
3. Pulled the job logs for the most recent scheduled nightly run
   (`ci-nightly-artifacts.yml`, run `30803686278`, 2026-08-03 09:59 UTC,
   all three jobs `success`) and found direct proof the fixed resolver is
   live in production CI, not just correct in isolation:
   - macOS job: `key: cef-runtime-darwin-cef-macos-arm64-148.23.23-codecs` →
     cache miss → downloaded → `verify-cef-framework-darwin: ✓ ... carries
     the BeginWindowDrag patch` → `Cache saved with key:
     cef-runtime-darwin-cef-macos-arm64-148.23.23-codecs`.
   - Linux job: `key: cef-runtime-cef-linux-x86_64-148.0.7778.180-codecs` →
     downloaded and cached under that key.
   - Windows job: `key:
     cef-runtime-windows-cef-windows-x86_64-148.0.7778.180` → cache hit
     (already warm from a prior run).

All three platforms' nightly artifacts have been building against
codec-enabled CEF since 2026-07-28, five days of green nightly runs before
this check.

## Takeaway

No code change was required this round — the fix already existed and was
already proven correct in prod. The value of this pass was closing an open
question with primary-source verification (live `gh` queries + actual CI job
logs) rather than trusting the "not yet remediated" note in a doc that had
gone stale in the ~6 days since it was written. Consistent with this
project's standing practice of verifying live state over trusting
static docs/assumptions before reporting a fix as done.

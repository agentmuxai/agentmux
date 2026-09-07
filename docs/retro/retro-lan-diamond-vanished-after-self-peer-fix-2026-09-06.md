# Retro: the status-bar LAN diamond vanished — because #3025 fixed the bug that was lighting it

**Date:** 2026-09-06
**Status:** Root-caused, no code change made yet. The current behavior matches
the written spec; what changed is the behavior users had actually learned. A
follow-up UI decision is proposed below but deliberately not implemented in
this retro.
**Severity:** Low functionally (LAN discovery is working correctly), Medium for
trust — the only status-bar signal that discovery is alive silently disappeared
for every single-instance user, with nothing to distinguish it from "off".
**Observed by:** repo owner, on a running v0.55.37 portable
**Related:** PR #3025 (`b904ae00d`, "fix(lan): stop this instance discovering
itself as a LAN peer"), `docs/specs/hostname-popover.md`

---

## TL;DR

The blue diamond (`◆`) next to the hostname is gated on **how many LAN peers
were found**, not on whether LAN discovery is **enabled**:

```tsx
// frontend/app/statusbar/HostPopover.tsx
const lanCount = () => lanInstances().length;
...
<Show when={lanCount() > 0}>
    <span style={{ color: "var(--accent-color)", "margin-left": "4px" }}>{"◆"}</span>
</Show>
```

Until #3025, every instance discovered **itself** as a phantom peer, so that
count was ≥ 1 whenever discovery was on. The diamond therefore *looked* like an
"LAN discovery is enabled" lamp, and that is how it was learned. #3025 removed
the phantom. On a machine with no other live AgentMux instance, the count is now
genuinely 0, so the diamond correctly hides — and the enabled state lost its only
status-bar representation.

Nothing is broken. The indicator's real meaning was simply masked by a bug for
as long as anyone had been looking at it.

---

## What was actually verified

Measured on the reporting instance (v0.55.37, channel `local-main-b28b7a`), not
inferred:

1. **Discovery is running, with no error.** Its srv log contains exactly two
   LAN lines and nothing else:
   ```
   "LAN discovery started (mDNS)", instance_id: "v0.55.37", port: 59859
   "LAN discovery enabled via setting"
   ```
   No `laninstances:error`, so the panel's "enabled" state is truthful and the
   mDNS daemon did start (this is *not* the Windows-firewall-blocked path).
2. **Zero peers, correctly.** No `LAN peer discovered` event has been logged by
   that instance at all. At the time of the report only two srv processes were
   alive — v0.55.37 itself and the v0.55.38 `task dev` instance — and the dev
   instance **never started LAN discovery**, because `settings.json` is now
   channel-isolated (`SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md`) and a
   fresh dev channel defaults `network:lan_discovery` to `false`. So there was
   genuinely nothing on the LAN to find. `lanCount() === 0` is the correct answer.
3. **The pre-fix behavior is still observable.** A v0.55.34 instance (older than
   #3025) that had been running earlier logged the phantom signature the #3025
   comment describes, verbatim:
   ```
   "LAN peer discovered", peer_id: "", address: fe80::142e:c4b3:9f29:8348, port: 55019
   ```
   — empty `peer_id`, its *own* port, its *own* link-local addresses, plus a
   conflict-renamed service (`agentmux-v0 (2).55.34._agentmux._tcp.local.`).
   That phantom is what used to hold the diamond on.
4. **The spec agrees with the code, not with the expectation.**
   `docs/specs/hostname-popover.md`: *"If mDNS is enabled **and peers are
   found**, the hostname text could show a subtle indicator: `Area54 ◆` … to
   signal **LAN peers exist**"*. Peer count, explicitly — never enabled-state.

---

## Root cause

Two correct-in-isolation decisions, whose interaction was never considered:

- **The indicator** was specified and built as "peers exist" (`lanCount() > 0`).
- **#3025** removed a phantom peer that made `lanCount()` ≥ 1 for *any* instance
  with discovery on, regardless of whether real peers existed.

Because the phantom was universal, the two were indistinguishable in practice
for the most common setup (one machine, one AgentMux). #3025 was a genuine bug
fix — the phantom cost a real fan-out request per `find_agent` lookup — and it
correctly did not touch the UI. But the fix silently changed what a user sees in
the single-instance case from "always lit when enabled" to "never lit", with no
release note connecting the two, and no other affordance in the status bar.

This is the recurring shape: **when a long-standing bug is the thing feeding a
UI signal, fixing the bug is a UI change.** The diff for #3025 contains no
frontend files, so nothing in review would have surfaced it.

---

## Why "no diamond" is worse than it sounds

The status bar now renders **identically** whether LAN discovery is enabled with
no peers, or disabled entirely. The only way to tell them apart is to open the
popover. For a feature whose entire value is passive background presence, losing
the passive signal removes the reason to trust it is on — which is precisely the
report that prompted this retro.

---

## Proposed fix (not implemented here)

Make the status bar distinguish the three real states, keeping the spec's
peers-exist meaning intact rather than overloading the diamond:

| State | Status bar |
|---|---|
| Discovery off | nothing (unchanged) |
| Discovery on, 0 peers | dimmed/outline `◇`, tooltip "LAN discovery on — no peers found" |
| Discovery on, N peers | current accent-colored `◆`, tooltip "N on LAN" |

`lanDiscoveryEnabled()` already exists in `HostPopover.tsx` immediately below
`lanCount()`, so this is a small, self-contained change. Deliberately left for a
separate PR with the repo owner's call on the glyph, since it is a visual-design
decision, not a defect fix.

An alternative — have the diamond track `lanDiscoveryEnabled()` — is **not**
recommended: it contradicts the spec, and it would discard the genuinely useful
"someone else is out there" signal that the diamond is supposed to carry.

---

## Lessons

1. **A bug can be load-bearing for a UI.** Before landing a fix that changes the
   population of a collection, grep for what reads that collection's `.length`.
   Here, `lanInstances().length` had exactly one other consumer, and it was a
   user-visible indicator.
2. **"Enabled" and "working" deserve separate signals.** Collapsing them is only
   safe while something guarantees they coincide — and here that guarantee was
   itself the bug.
3. **The empirical check was cheap and decisive.** Two greps of the srv log
   ("did discovery start?", "were any peers ever discovered?") separated
   "feature broken" from "feature idle" in under a minute, and disproved the
   firewall-blocked hypothesis without touching the running instance.

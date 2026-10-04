# Report: "other machines can't see this one", and the window shifting sideways

**Status:** analysis — the page shift is fixed in this PR (§3); the LAN fault is not, and its open questions are in §2.4
**Date:** 2026-10-04
**Verified against:** `a5ed16855` (main); the running app was a 0.59.5 portable build

---

## 1. What the owner saw

1. On a Windows machine (wired, Private network profile) the Host popover showed:
   > ⚠ Other machines can't see this one, even though it may see them. Another program may be using the mDNS port (UDP 5353). AgentMux keeps retrying on its own (2 so far); turning LAN off and on retries at once.
2. While an agent worked on it, the whole AgentMux window shifted left by about 100 px: the left edge of the left pane and the right end of the status bar went off screen.

The two turned out to be unrelated. They are reported together because the second was found while working on the first.

## 2. LAN: announced over IPv6 only

This is the same fault recorded in `docs/specs/SPEC_LAN_FIREWALL_SETUP_2026_10_01.md` §8.1 and §8.3. What is new is that, since #4196, the mDNS library's own messages reach the srv log, which neither earlier investigation had.

### 2.1 What the warning means

`lan_mdns_health.rs` turns the mDNS daemon's `Announce` events into a verdict. "Undiscoverable" means the daemon announced on **none** of the host's IPv4 addresses. Peers browse over IPv4, so they never hear this host, while it still hears them. The watchdog then rebuilds the daemon on a backoff (30 s, 1 min, 2 min, 5 min, then 10 min).

### 2.2 Findings

| Finding | Basis |
|---|---|
| **Not a bind failure**, although the warning says another program may hold the port | The watchdog's own diagnosis on every attempt: the IPv4 socket set-up works on both IPv4 adapters. srv held UDP 5353 in both address families at the time |
| UDP 5353 is shared, as designed | Holders: the Windows DNS client, this srv, a browser, and a second AgentMux on the same machine (a `task dev` build) |
| The library does try IPv4 | It logged announcement attempts on the Ethernet adapter's IPv4 address, yet the health verdict counted no IPv4 announcement |
| Something else answers for this machine's name | The library's probe for the machine's own `.local` host name ended with a rename to `<name>-2.local`, and its conflict handler fired repeatedly on answers for that name arriving on the IPv4 socket |
| The host kept seeing its peers | Peer discoveries throughout the window |
| Peers could see the host, at least some of the time | After the second AgentMux was stopped, the owner saw the host and its channels from another machine, while the watchdog was still flagging it |

What was tried: stopping the second AgentMux on the machine (its `task dev` launcher restarted it until the launcher itself was stopped; see §2.6), then switching LAN off and on. Peers were discovered again at once, but the watchdog raised the same verdict about 30 s later and resumed its rebuilds.

Two steps need an elevated session and were not done: turning off Windows' own mDNS responder, which is on and is the likeliest other responder for the machine's name, and adding a Windows Firewall rule for the portable build, which runs from a new path for every build.

### 2.3 Two readings

1. **The detector gives a false alarm on this machine.** Peers see the host, yet no IPv4 `Announce` event is counted. This would happen if the library sends on IPv4 without emitting `DaemonEvent::Announce` for that interface.
2. **Visibility flaps.** Each rebuild tears the daemon down and starts a new one, so a peer can lose the host briefly at each attempt, and some attempts may leave it announcing only over IPv6.

The rebuild loop matters in both readings: on a false alarm it causes the flapping it is meant to cure.

### 2.4 Likely causes, most likely first (none proven)

1. **A name clash on the machine.** Windows' mDNS responder (and, until it was stopped, the second AgentMux) answers for the machine's `.local` name. The library renames its host record and logs repeated conflicts on the IPv4 socket. How that stops the IPv4 announcement, or its event, is not established.
2. **`mdns-sd` 0.12 on IPv4 interfaces.** §8.3 already suspected its late-interface path (`add_new_interface`); here the IPv4 announce path runs, but no IPv4 address reaches the health verdict.
3. **No firewall rule for the executable.** It doesn't explain the missing announcement (sending is outbound), but it would drop peers' questions even with announcing fixed.

### 2.5 Recommendations

1. Correct the warning text: when every interface's set-up works, "another program may be using the mDNS port" is the wrong lead. Say that the daemon is not announcing on IPv4, and give the retry count.
2. Before rebuilding, cross-check the verdict against evidence that peers see this host (for example a peer's record of us in the names-only LAN feed, #4297), so a false alarm does not tear down a working daemon every few minutes.
3. Test with Windows' mDNS responder off (needs admin) to settle cause 1.
4. Try a newer `mdns-sd`, or log per-interface announce events in the health module's own line, to settle cause 2.
5. Portable builds need a firewall rule that survives a new path (the subject of `SPEC_LAN_FIREWALL_SETUP_2026_10_01.md` §4.1).

### 2.6 Side finding: `task dev`'s launcher restarts a stopped app

Killing a `task dev` instance's processes is not enough: its `agentmux-launcher.exe` (under `dist/cef-dev-<stamp>/`) started the app again within seconds. Stop the launcher first. The `run` skill's cleanup section now says so.

## 3. The window shifting sideways

### 3.1 Measurement

With the window 630 px wide, opening the Host popover was enough to reproduce it:

| Element | `x` before | `x` after |
|---|---|---|
| `.window-header` | 0 | −97 |
| `.status-bar` | 0 | −97 |

The status bar's left group was **721.5 px wide, starting at x = 6**: its right edge at 727.5 px overflowed the window by 97.5 px, the size of the shift. Focusing the agent pane's composer, whose left edge was then off screen, scrolled it back to 0.

### 3.2 Mechanism

- `.status-bar-left` (backend uptime, the system stats, GPU) is `flex: 0 1 auto` with its default `min-width: auto`, so it cannot shrink below its content. The status bar already moves its *right* group to a second row when space runs out (`flex-wrap`), but nothing lets the left group itself fit, so in a narrow window, or at a larger chrome zoom, the page is wider than the window.
- `body` has `overflow: hidden` and `transform: translateZ(0)` (`app.scss`). `overflow: hidden` hides the overflow but leaves `body` a scroll container: a focus or `scrollIntoView` can still scroll it, and nothing scrolls it back. The transform also makes `body` the containing block of every `position: fixed` element, so portaled popovers live inside that scroller.

Measured in headless Chrome 154 (the app's Chromium), a 630 px window, a status group 727 px wide:

| Setup | Hidden overflow | `body.scrollLeft` after `scrollIntoView` |
|---|---|---|
| Today: `body { overflow: hidden }` | 113 px | **113** |
| `body { overflow: clip }` | 113 px | 0 |
| Left group with `min-width: 0` | 0 | 0 |

(113 rather than 97 because the test window's inner width differs.) Which call scrolled `body` in the app was not identified; any focus or `scrollIntoView` reaching into the overflow does it, and both fixes below stop every such call.

### 3.3 Fix (this PR)

1. `html` and `body`: `overflow: clip` instead of `hidden`. Clipped exactly as before, but no longer a scroll container, so nothing can scroll the app sideways.
2. `.status-bar`: `overflow: clip`, so a status group still too wide for the window is cut at the edge
   instead of widening the page. The left group does not wrap or shrink: an earlier version of this fix
   gave it `flex-wrap: wrap` and `min-width: 0`, which stacked its items onto up to four rows, so that
   was removed and the bar looks as it always did.

Not changed: `body`'s `transform` (it exists for compositing; see its comment), and the status bar's existing two-row wrap.

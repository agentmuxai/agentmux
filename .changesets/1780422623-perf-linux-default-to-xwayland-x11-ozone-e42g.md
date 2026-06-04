---
type: patch
---

perf(linux): default to XWayland (X11 ozone) — 5–8× fewer frame stalls

Linux CEF 146 (Chromium 146) on Mutter (GNOME) has broken native-Wayland
GPU buffer negotiation — Chromium logs
`WaylandZwpLinuxDmabuf::OnTrancheFlags Not implemented` at startup and
then responds `LayerTreeHostImpl::DidNotProduceFrame` to ~89 % of
Mutter's `BeginFrame` requests. The renderer's `requestAnimationFrame`
callbacks (including predictive local echo's render path, #1223) are
gated on those frames, so typing visibly hangs and pumps out on key
release.

Setting `--ozone-platform=x11` routes the renderer through XWayland's
X11 present path — the wire-format Linux Chromium has shipped
reliably for years. Measured locally on the same host
(`scripts/capture-trace-ipv6.cjs` + CDP `Profiler.start`, 10 panes,
sustained held key):

|                 | Wayland (native) | XWayland (this PR) |
| --------------- | ---------------- | ------------------ |
| rAF firing rate | 2.5 Hz           | **6.4 Hz**         |
| rAF gap p50     | 138 ms           | 136 ms             |
| rAF gap p95     | 1182 ms          | **224 ms**         |
| rAF gap max     | 8280 ms          | **1024 ms**        |

p50 is unchanged (the 136 ms median is the residual per-frame Blink/CC
compositor cost — separate work). p95 and worst-case drop **5×** and
**8×**: no more "hold a key, nothing happens, release, dump." VSCode
on the same machine (Electron 39 / Chromium 142) sits on the XWayland
path by default and runs smoothly here — this PR brings the AgentMux
runtime onto the same well-trodden path until native Wayland is fixed
upstream.

`AGENTMUX_OZONE_PLATFORM=wayland` opt-out remains for regression
testing the native-Wayland path (which will be revisited once the CEF
148 binary distribution lands for Linux — the source bump is already
in main, #1221, but the patched libcef.so needs a rebuild).

Not a complete fix for full VSCode parity — the residual 136 ms median
is per-frame compositor work that still needs CSS layer-tree audit
follow-ups — but this is the largest single Linux user-visible win
since predictive echo (#1223) and removes the worst pathological
stalls.



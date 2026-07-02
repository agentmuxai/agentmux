# PLAN: Linux Window Transparency — AgentU execution guide

**Date:** 2026-07-01
**Owner:** AgentU (Linux/ubuntu agent)
**Written by:** AgentO (macOS agent) — questions → jekt AgentO or leave PR comments
**Read first:** `docs/specs/SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01.md` (the two-track model)
**Status of the other half:** macOS Track 1 merged-in-flight (agentmux PR #1895, verified working);
the shared CEF patches are on fork PR agentmuxai/cef#4 (branch `agento/7778-renderer-transparency`).

This plan is deliberately prescriptive — follow it step by step; the exact code to add is
included. Do not redesign; if something doesn't apply cleanly, stop and report rather than
improvise.

---

## What you are delivering (two independent tasks)

| Task | What the user sees | Depends on |
|---|---|---|
| **L1 — uniform window alpha (X11)** | The whole AgentMux window fades over the desktop when `window:opacity` < 1 — identical to what Windows and macOS now ship | Nothing. Host-Rust only. Works with ANY libcef. |
| **L2 — rebuild patched libcef.so** | Pane *interiors* become truly transparent (per-pixel “glass”) — today only borders/gaps are | Fork PR agentmuxai/cef#4 (already pushed) |

Do L1 first (small, self-contained, immediately shippable), then L2.

---

## Task L1 — `_NET_WM_WINDOW_OPACITY` uniform alpha

### Context you need (do not re-derive)

- `set_window_transparency` / `set_window_opacity` in
  `agentmux-cef/src/commands/window/transparency.rs` are implemented for Windows
  (`WS_EX_LAYERED`) and macOS (`NSWindow.alphaValue`, PR #1895). The Linux branch is a
  no-op. You are filling it in with the X11 EWMH mechanism.
- AgentMux on Linux runs ozone **X11/XWayland by default** (`docs/linux.md`);
  `_NET_WM_WINDOW_OPACITY` is honored by Mutter, KWin, picom, xfwm4 — including for
  XWayland clients. Native-Wayland ozone (opt-in via `AGENTMUX_OZONE_PLATFORM=wayland`)
  has **no** uniform-alpha protocol → log once and no-op there.
- Use PR #1895's diff as your structural template: a `wrap_task!` UI task in
  `agentmux-cef/src/ui_tasks.rs` + arms in `transparency.rs`. Mirror it exactly.

### Step 1 — dependency

In `agentmux-cef/Cargo.toml`, add (in the linux target-dependency section; create one if
missing, next to where other target deps live):

```toml
[target.'cfg(target_os = "linux")'.dependencies]
x11rb = "0.13"
```

### Step 2 — UI task in `agentmux-cef/src/ui_tasks.rs`

Add directly below the `SetWindowAlphaTask` (macOS) section — same shape:

```rust
// ── Window alpha (Linux/X11 uniform whole-window opacity) ────────────────
// Track 1 of SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01, Linux arm: the EWMH
// analogue of Win32 LWA_ALPHA / NSWindow.alphaValue. The compositor (Mutter,
// KWin, picom, xfwm4) fades the finished window over the desktop — including
// under XWayland, which is AgentMux's default ozone platform. Post-render:
// needs no CEF/renderer cooperation. Native-Wayland ozone has no equivalent
// protocol; there we log once and no-op (per-pixel Track 2 is the only route).

#[cfg(target_os = "linux")]
wrap_task! {
    pub struct SetWindowAlphaTask {
        state: Arc<AppState>,
        label: String,
        alpha: f64,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(label = %self.label, "[opacity] SetWindowAlphaTask: no window for label");
                return;
            };
            // Under ozone-x11 this is the X11 Window XID. Under native
            // Wayland it is not an XID — guarded by the env check below.
            if std::env::var("AGENTMUX_OZONE_PLATFORM").as_deref() == Ok("wayland") {
                tracing::warn!("[opacity] uniform window alpha unsupported on native Wayland (no protocol); use per-pixel transparency");
                return;
            }
            let xid = window.window_handle() as u32;
            if xid == 0 {
                tracing::warn!(label = %self.label, "[opacity] SetWindowAlphaTask: null X11 window handle");
                return;
            }
            match x11_set_window_opacity(xid, self.alpha) {
                Ok(()) => tracing::info!(label = %self.label, alpha = self.alpha, "[opacity] applied _NET_WM_WINDOW_OPACITY"),
                Err(e) => tracing::warn!(label = %self.label, "[opacity] X11 property set failed: {e}"),
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub fn post_set_window_alpha(state: &Arc<AppState>, label: &str, alpha: f64) {
    let mut task = SetWindowAlphaTask::new(state.clone(), label.to_string(), alpha);
    post_task(ThreadId::UI, Some(&mut task));
}

/// Set (or clear, when alpha >= 1.0) the EWMH `_NET_WM_WINDOW_OPACITY`
/// CARDINAL/32 property on the toplevel client window. Value is
/// alpha × 0xFFFFFFFF. Modern compositors read it from the client window.
#[cfg(target_os = "linux")]
fn x11_set_window_opacity(xid: u32, alpha: f64) -> Result<(), Box<dyn std::error::Error>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};

    let (conn, _screen) = x11rb::connect(None)?;
    let atom = conn
        .intern_atom(false, b"_NET_WM_WINDOW_OPACITY")?
        .reply()?
        .atom;
    if alpha >= 1.0 {
        conn.delete_property(xid, atom)?.check()?;
    } else {
        let value = (alpha.clamp(0.0, 1.0) * u32::MAX as f64) as u32;
        conn.change_property32(PropMode::REPLACE, xid, atom, AtomEnum::CARDINAL, &[value])?
            .check()?;
    }
    conn.flush()?;
    Ok(())
}
```

NOTE: if `window.window_handle()` on Linux returns a wider type, cast via
`as u64 as u32` won't truncate real XIDs (XIDs are 29-bit). Check the existing usage in
`browser_panes.rs` for the exact type and match it.

### Step 3 — arms in `agentmux-cef/src/commands/window/transparency.rs`

Mirror the macOS arms from PR #1895 **exactly**, with `target_os = "linux"`:

1. In `set_window_transparency`, after the macOS block:

```rust
    // Linux — Track 1 (SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01): EWMH
    // _NET_WM_WINDOW_OPACITY, the X11/XWayland analogue of the Win32 layered
    // window and NSWindow.alphaValue arms above.
    #[cfg(target_os = "linux")]
    {
        let alpha = if transparent { opacity.clamp(0.0, 1.0) } else { 1.0 };
        crate::ui_tasks::post_set_window_alpha(state, &label, alpha);
    }
```

Then CHANGE the final suppression line from
`#[cfg(not(any(target_os = "windows", target_os = "macos")))]` to remove it entirely
(all three platforms now use the args) — delete both that cfg line and its
`let _ = (transparent, opacity);`, and update the module-header comment's
"Linux — not yet implemented" line.

2. In `set_window_opacity`, after the macOS event loop, add the same loop with
`#[cfg(target_os = "linux")]`, calling `post_set_window_alpha(state, ev_label, *ev_opacity as f64)`
for `WindowOpacityApplied` and `post_set_window_alpha(state, ev_label, 1.0)` for
`WindowOpacityCleared`. **Handle BOTH arms** — reagent P1 on #868: Applied-only left
windows stuck translucent.

### Step 4 — build + verify

```bash
cargo check -p agentmux-cef          # must pass before anything else
task dev                             # branch-keyed dev instance
```

Set in the dev instance's `~/.agentmux/dev/<branch>/settings.json`:
`"window:transparent": true, "window:opacity": 0.5` (hot-reloads; no restart).

Verification (ALL required — the #947 lesson is that bots can't see this feature):
- `xprop -name "AgentMux" _NET_WM_WINDOW_OPACITY` (or `-id <xid>`) → shows the CARDINAL.
- Eyes: whole window fades over the desktop. Drag the opacity through several values.
- Set `"window:transparent": false` → property deleted, window fully opaque again.
- Host log: `[opacity] applied _NET_WM_WINDOW_OPACITY` lines follow your changes.
- Test on GNOME/Mutter (your box). KWin if available. Both are optional-nice, Mutter is required.

### Step 5 — ship

```bash
LC_ALL=C task changeset -- minor "feat(linux): window transparency/opacity via _NET_WM_WINDOW_OPACITY (Windows/macOS parity)"
```
Commit (NO version bumps — changesets only), push branch `agentu/linux-transparency-track1`,
PR referencing the spec + PR #1895 as the sibling. reagent reviews automatically on push.
**If `package-lock.json` shows a diff you didn't make (dropped `peer: true` lines): `git checkout -- package-lock.json` — do NOT commit it** (npm-version regen noise; this exact trap contaminated the old transparency branch).

---

## Task L2 — rebuild patched libcef.so with the new renderer patch

### What changed and why you're rebuilding

Fork PR **agentmuxai/cef#4** (branch `agento/7778-renderer-transparency`, 2 commits on top
of `2720ba103`) adds the **renderer-side Blink base-background-color override** — the
confirmed root cause fix from `docs/retro/cef-linux-transparency-consolidated.md` §"The
patch (not yet implemented)". This is the ONE remaining blocker for pane-interior
transparency on Linux. (The second commit is macOS-only — `patch/patches/
mac_rwhv_transparent_background.patch` — it rides along harmlessly in your build.)

### Steps

1. Get PR #4 merged into `agentmux/7778-drag-rightclick-and-transparency` first (reagent
   review), or build from the PR branch directly if the user says go.
2. On your Linux build box, follow **`docs/cef-build/build-patched-libcef.md`** exactly as
   for `148.0.20-2`, except checkout `agento/7778-renderer-transparency` (or the canonical
   branch post-merge). Remember from that doc: `is_official_build=true`, `patcher.py` is
   NOT idempotent (reset first), verify runs pre-strip.
3. The mojom change (`cef.mojom` + `NewBrowserConfig.background_transparent`) regenerates
   bindings automatically during the build — no manual step.
4. Release as **`cef-linux-x86_64-148.0.20-3`** on agentmuxai/cef (same asset shape as
   `-2`). Note: the release TAGS on this repo point at a meaningless commit — put the real
   source commit SHA in the release NOTES (this is the provenance record; see spec §1.3).
5. Wire it: wherever `148.0.20-2` is referenced (`release.yml`, `build-linux.yml`,
   `AGENTMUX_CEF_RUNTIME_DIR` staging on your box), bump to `-3`.

### Verification protocol (pixel-level, pane INTERIORS)

Baseline before your rebuild (with `-2`): window borders/tab-bar gaps show the desktop;
pane interiors are opaque `rgb(62,62,62)`-ish and do NOT react to `window:opacity`.

With `-3` + `window:transparent=true`:
- [ ] Set a saturated (e.g. solid green) wallpaper.
- [ ] Sample a pane-INTERIOR pixel (terminal pane background, agent pane background) —
      it must show wallpaper bleed (green tint), not neutral gray.
- [ ] `window:opacity` 0.85 → 0.25 visibly changes pane interiors, not just gaps.
- [ ] Opaque mode (`window:transparent=false`) is pixel-identical to today (no white
      flash on startup, no washed-out panes) — this catches the override leaking into
      non-transparent windows.
- [ ] Embedded **browser panes** (Browser widget showing an external site) still render
      the site normally — sites without an explicit body background must NOT become
      see-through. (The patch gates on per-browser background_color alpha=0; browser
      panes are created opaque `0xFF000000` in `browser_pane/creation_views.rs:134` and
      must stay unaffected.)
- [ ] Test on **both** default X11/XWayland AND `AGENTMUX_OZONE_PLATFORM=wayland`.
- [ ] Floaters/secondary windows (tear off a pane) inherit the transparency (#1313 path).

### Do NOT re-investigate (documented dead ends — consolidated retro §"Dead ends")

Wayland protocol "not implemented" errors (red herrings) · `wl_surface_set_opaque_region`
suppression · `contents_opaque` flips in cc · `UpdateBaseBackgroundColor` re-push ·
`--disable-lcd-text` variations · brute-force raster/tile patches · Chad Nelson's Views
patch alone · LD_PRELOAD shims. If pane interiors are STILL opaque after `-3`, the next
step is the **promoted-layer ladder** in the spec §4.3 — report first, don't dig alone;
AgentO is running the same experiment on macOS and findings transfer.

---

## Reporting

When done (or blocked), update:
- agentmux PR for L1; fork PR #4 thread for L2 results.
- The spec's §7 sequencing table (check off Linux items).
- If pane interiors work: close the loop on issues #301/#828/#872 references and note it
  in `docs/retro/cef-linux-transparency-consolidated.md` (add a dated postscript).

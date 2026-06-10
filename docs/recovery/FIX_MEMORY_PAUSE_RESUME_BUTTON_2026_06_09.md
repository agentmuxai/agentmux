# Fix — Low-memory "Resume" button is inert (HTML quoting bug)

**Status:** P0 fix implemented (this PR); P1 (auto-resume) designed below as follow-up
**Date:** 2026-06-09
**Author:** AgentC
**Related:** `docs/specs/SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md` (Phase 1b)

---

## 1. Symptom

A long-lived AgentMux window (v0.42.0, ~4 days uptime) was OOM-paused after the *host*
hit 99% commit-limit exhaustion, showing the gated-recovery **"Paused — system memory low"**
page. Clicking **Resume did nothing**, even after system memory was freed — the window was
permanently wedged. Recovery required an operator to drive the renderer back to the app URL
over the Chrome DevTools Protocol (`Page.navigate` on `:9222`).

## 2. Root cause — malformed `onclick`

`memory_paused_page()` (`agentmux-cef/src/client/mod.rs`) builds the Resume button as:

```rust
<button class="primary" onclick="location.href = {app_url_js}">Resume</button>
```

`app_url_js = js_string_literal(app_url)` (`agentmux-cef/src/client/helpers.rs`) returns a
**double-quoted** JS string and is documented as a `<script>`-context literal (it escapes
`<`/`>`/`&` to defend against `</script>` injection). Dropped into a **double-quoted HTML
attribute**, the quotes collide and the rendered markup is:

```html
<button class="primary" onclick="location.href = "http://127.0.0.1:63627/?ipc_port=63627&ipc_token=…">Resume</button>
```

The attribute terminates at the second `"` (before `http`), leaving `onclick="location.href = "`
(a no-op) and turning the URL into stray markup. **The click runs nothing.** The target URL is
valid — the app's pre-warmed pool windows load from the same origin successfully.

Consequence: the gated-recovery pause page has **never** been resumable via its button. Anyone
who hits a low-memory renderer OOM is wedged until the process is killed.

## 3. The fix (P0 — this PR)

Use `js_string_literal` in the context it's built for — a `<script>` block — instead of an
inline HTML attribute. The handler is attached by element id via `addEventListener`:

```rust
<button id="amx-resume" class="primary">Resume</button>
<button id="amx-quit">Quit this window</button>
...
<script>
(function(){
  var r = document.getElementById('amx-resume');
  if (r) r.addEventListener('click', function () { location.href = {app_url_js}; });
  var q = document.getElementById('amx-quit');
  if (q) q.addEventListener('click', function () { window.close(); });
})();
</script>
```

This is quoting-safe (the literal lives in script context, exactly what its `<`/`&`
escaping is for) and keeps the same UX.

**Regression test** (pure string, no CEF runtime): `memory_paused_page_resume_button_has_working_handler`
asserts the inert `onclick="location.href` antipattern is gone, the URL appears as a valid JS
string literal (`location.href = "http://…&…"`), and the handler is wired via
`addEventListener`.

### Same bug, second instance (also fixed in this PR)

ReAgent review flagged the identical pattern on the **broken-install error page**:
`assets_missing_data_url()` in `agentmux-cef/src/commands/window/creation.rs` put
`js_string_literal(&detail)` into a double-quoted `onclick="navigator.clipboard && …writeText(…)"`,
making the **"Copy path"** button permanently inert. Fixed the same way — extracted a testable
`assets_missing_html()`, moved the handlers into a `<script>` block via `addEventListener`, and
added `copy_path_button_has_working_handler`.

Both data:-page buttons in the CEF host are now wired in script context; no remaining
`onclick="…{js_string_literal}…"` attribute call sites.

## 4. Follow-up (P1 — memory-gated auto-resume, separate PR)

P0 restores the *manual* one-click path. Full automation — so no human/agent is ever needed —
is SPEC_GATED_RENDERER_RECOVERY **Phase 1b**, and the codebase already has every piece:

- **Track paused windows by label** (not by Browser handle). Per `ui_tasks.rs`'s rule
  ("don't pass Browser/Window across threads — pass `Arc<AppState>` and look up on the UI
  thread"), record `{window_label → recovery_url}` in `AppState` when `memory_paused_page` is
  shown (in `on_render_process_terminated`, which already has `self.state` and resolves the
  label). Clear the entry in `on_before_close` / on successful resume.
- **Trigger from the memory heartbeat.** `memory_heartbeat.rs` already runs a 20 s loop and
  publishes `COMMIT_FREE_MB` via `commit_free_mb()`. When the paused set is non-empty **and**
  commit-free has stayed above a resume threshold for N consecutive samples (hysteresis — e.g.
  `≥ 2 × RESUME_FLOOR_MB` for 2–3 samples to avoid flapping), post a UI task per paused label.
- **Resume on the UI thread** by reusing the existing pattern: look up the browser with
  `state.get_browser(label)` and navigate via a `DeferredLoadUrlTask`-style task to the stored
  recovery URL.
- **Bounding:** reuse `MEMORY_PAUSE_BUDGET` / `MEMORY_PAUSE_WINDOW` convergence so a window that
  immediately re-OOMs still falls through to the give-up page instead of looping.

Wiring requires passing `Arc<AppState>` to the heartbeat (or a small dedicated resume subsystem
holding it) and a host build to validate the threading — hence a separate PR.

### Other layers (tracked elsewhere, not code in these PRs)
- **Proactive shedding:** release the pre-warmed window pool under rising commit pressure before
  the OS OOM-kills a live renderer.
- **Renderer-free paused overlay:** render the paused state natively so the recovery UI itself
  can't be OOM-killed under total exhaustion (also Phase 1b).
- **System-level:** the real trigger was host-wide commit exhaustion from a long-uptime driver
  leak + accumulated stale instances — a commit watchdog and stale-instance reaping belong at the
  launcher/OS layer, independent of this page.

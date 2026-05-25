# Research: `OnGetAuthCredentials` silently suppressed under CEF Chrome runtime

**Date:** 2026-05-25
**Author:** AgentA (Claude Opus 4.7)
**Symptom:** Browser pane navigating to a 401-protected URL (e.g. `https://pulse.asaf.cc` returning `WWW-Authenticate: Basic realm="Secure Area"`) shows `ERR_INVALID_AUTH_CREDENTIALS (-338)` and falls back to the AgentMux error page. **No** HTTP Basic auth modal appears. PR #906 + four follow-ups shipped the modal end-to-end, so the code path is wired.
**Bug:** `RequestHandler::on_auth_credentials` is **never invoked** by CEF for this request — confirmed via the instrumentation merged in PR #1035 (`[browser-pane-auth][ENTRY]` absent from the host log; `[load-error][ENTRY]` for the same URL fires).

---

## TL;DR

CEF 146 only ships the **Chrome runtime** (the Alloy runtime was removed in 2024). The Chrome runtime owns its own native login-prompt UI for HTTP Basic/Digest auth challenges and intercepts the challenge **before** the embedder's `OnGetAuthCredentials` callback gets to see it. The result: the embedder's auth-modal flow is silently dead.

The documented fix is to pass `--disable-chrome-login-prompt` (or equivalently `--disable-features=ChromeLoginPrompt`) as a Chromium command-line switch. This makes the Chrome runtime defer the auth challenge to the embedder, so `OnGetAuthCredentials` fires as the CEF API documents.

CEF tracks this as [issue #3603](https://github.com/chromiumembedded/cef/issues/3603) ("chrome: GetAuthCredentials requires --disable-chrome-login-prompt"). The CEF4Delphi binding sets the switch by default for the same reason ([CEF4Delphi#520](https://github.com/salvadordf/CEF4Delphi/issues/520)).

We don't pass this switch. That's the bug.

---

## 1. What we tried + what the trace showed

### 1.1 Instrumentation PR #1035 (merged 2026-05-25)

Added unconditional `tracing::info!` calls at the entry of `on_auth_credentials` and `on_load_error` in `agentmux-cef/src/client/mod.rs`. Goal: confirm whether CEF calls `on_auth_credentials` at all.

### 1.2 Reproduction

1. `task dev` with the instrumented binary.
2. Browser pane navigates to `https://pulse.asaf.cc`.
3. Result in host log (`~/.agentmux/dev/main/logs/agentmux-host-v0.38.3.log.2026-05-25`):
   ```json
   {"timestamp":"2026-05-25T13:53:12.865493Z","level":"INFO",
    "fields":{"message":"[load-error][ENTRY] url=\"https://pulse.asaf.cc/\" 
      error=\"ERR_INVALID_AUTH_CREDENTIALS\" (-338) is_main_frame=true aborted=false"}}
   ```
   **No `[browser-pane-auth][ENTRY]`** anywhere in the file.

4. Cleared the HTTP cache (`~/.agentmux/dev/main/cef-cache/Default/Cache/`), retried — same result. Cache theory ruled out.

### 1.3 Definitive signal

CEF is receiving the 401 response (the load-error fires with `ERR_INVALID_AUTH_CREDENTIALS`, which is Chromium's "tried, no creds available, giving up" code). It then **does not** invoke our `OnGetAuthCredentials` callback before raising the error. That is the discriminator between every other theory and this one — see §3.

---

## 2. Root cause (from the CEF source-of-truth)

### 2.1 Chrome runtime intercepts auth before the embedder

Modern Chromium ships an in-process "Chrome login prompt" UI for HTTP Basic / Digest challenges. The Chrome runtime in CEF inherits this UI. When a 401 comes back:

1. Chromium's Network Service raises `OnAuthRequired` to its `URLLoaderClient`.
2. The browser process's `LoginHandler` decides whether to show the Chrome-native login dialog OR delegate to the embedder.
3. By default (with Chrome runtime), it shows the Chrome-native dialog. Our embedder's `RequestHandler::OnGetAuthCredentials` is never called.

Per CEF issue [#3603](https://github.com/chromiumembedded/cef/issues/3603), the Chromium switch that turns off the in-process dialog and re-enables embedder delegation is:

```
--disable-chrome-login-prompt
```

This is implemented as a Chromium feature flag (`ChromeLoginPrompt`), which can also be disabled via:

```
--disable-features=ChromeLoginPrompt
```

Either form works. The switch must be set BEFORE `cef_initialize`, via `OnBeforeCommandLineProcessing` (our `app.rs`'s `on_before_command_line_processing` is the right place).

### 2.2 Why the AgentMux portable doesn't show the Chrome-native login either

Important nuance: in our dev/portable build the **Chrome-native login dialog also doesn't appear**, even though the in-process flow is what suppresses our callback. Two hypotheses for why we go straight to `ERR_INVALID_AUTH_CREDENTIALS` instead of showing the Chrome dialog:

- **A. Frameless window quirk.** Our main window is frameless (`is_frameless = true`) with a transparent background (`background_color = 0x00000000`). The Chrome login prompt is a Chrome native dialog that may not find a suitable parent and fail silently. Reported variant in [CEF forum thread](https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=18608) on related frameless-mode prompt suppression.
- **B. CefBrowserView path.** Browser panes are created via `CreateBrowserView` (rather than `CreateBrowser` with a parent HWND). The login-prompt UI might not be wired into the BrowserView path.

Either way, the fix is the same: disable the Chrome-side flow, route auth back to our embedder. We already have the modal UI built — we just need CEF to ask us.

### 2.3 Other suppression vectors (ruled out)

| Theory | Verdict |
|---|---|
| CEF cache holds a failed-auth entry | Ruled out — cleared `~/.agentmux/dev/main/cef-cache/Default/Cache`, retried, same result |
| `CefRequestContext` mismatch (handler not registered against the pane's context) | Unlikely — `on_load_error` IS firing for the same request and uses the same handler family. If the request context were wrong, neither would fire |
| `auth_credentials` method name not wired to CEF vtable | Verified against cef-rs v146.7.0 source: `bindings/x86_64_pc_windows_msvc.rs:26815` exposes `fn auth_credentials(…)` and line 26889 wires it to `get_auth_credentials`. Our handler in `handlers.rs:336` matches |
| Top-level navigation auth bypass (Chromium policy) | The CEF docs say `OnGetAuthCredentials` IS called for navigation auth challenges. The bypass for top-level navigations only applies when no embedder handler exists OR Chrome-runtime-prompt intercepts (this case) |

---

## 3. The fix

Add the disabling switch in `agentmux-cef/src/app.rs`'s `on_before_command_line_processing`:

```rust
// HTTP Basic / Digest auth — route the challenge to the embedder's
// `RequestHandler::on_auth_credentials` callback (which surfaces our
// BrowserAuthModal) instead of letting the Chrome runtime show its
// own in-process login dialog. Without this, CEF 146 silently
// consumes the challenge and the request fails with
// ERR_INVALID_AUTH_CREDENTIALS — the embedder callback is never
// invoked, so the modal we built in PR #906 + follow-ups stays
// dormant. See:
//   docs/research/RESEARCH_CEF_AUTH_CALLBACK_SUPPRESSED_2026_05_25.md
//   https://github.com/chromiumembedded/cef/issues/3603
//   https://github.com/salvadordf/CEF4Delphi/issues/520
cmd.append_switch(Some(&CefString::from("disable-chrome-login-prompt")));
```

One line, zero risk. The switch is a CEF-supported flag.

---

## 4. Verification plan

After the fix lands and `task dev` rebuilds:

1. Navigate browser pane → `https://pulse.asaf.cc`.
2. Tail `~/.agentmux/dev/main/logs/agentmux-host-v0.38.3.log.2026-05-25` for `[browser-pane-auth][ENTRY]`.
3. **Expected**: the entry log fires, then the standard `[browser-pane-auth][...]` "auth-required origin=…" line fires, then the renderer surfaces the auth modal.
4. Enter creds → the auth completes via the `browser_pane_auth_submit` IPC path that PR #906 wired.

The instrumentation merged in PR #1035 remains valuable as a permanent diagnostic; it'll continue to log every auth challenge and every load error for any future investigation.

---

## 5. Related — `on_load_error` shows the wrong page for browser panes

Separate bug observed during this investigation: `agentmux-cef/src/client/mod.rs:1120` `on_load_error` renders the AgentMux "Failed to load AgentMux frontend / Make sure the Vite dev server is running" data-URL fallback page for **any** main-frame load error, including browser pane navigation failures (this is what the user saw on pulse.asaf.cc).

That message is intended only for the host-frontend's failure to reach `http://localhost:5173`. Browser panes should show either:
- CEF's default chrome error page (NET::ERR_*), or
- A pane-specific error UI that says "Failed to load <url>" without the Vite hint.

Not blocking the auth fix. Will be addressed in a follow-up PR once auth works (so the auth path doesn't briefly route through the wrong error page mid-flow).

---

## 6. Sources

- [CEF issue #3603 — chrome: GetAuthCredentials requires --disable-chrome-login-prompt](https://github.com/chromiumembedded/cef/issues/3603)
- [CEF4Delphi issue #520 — CEF 126 proxy ERR_INVALID_AUTH_CREDENTIALS](https://github.com/salvadordf/CEF4Delphi/issues/520) (sister binding's identical workaround)
- [CEF Forum — GetAuthCredentials not been invoked](https://magpcss.org/ceforum/viewtopic.php?f=6&t=14892)
- [CEF Forum — CEF Chrome Runtime: Disable Save Password prompt](https://magpcss.org/ceforum/viewtopic.php?f=6&t=18608) (related Chrome-runtime UI suppression patterns)
- [cef-rs binding source](https://crates.io/crates/cef) — `bindings/x86_64_pc_windows_msvc.rs:26815` (method name verification)
- `agentmux-cef/src/client/mod.rs:1442` — the `on_auth_credentials` handler we want to fire.
- `agentmux-cef/src/app.rs:348` — the `on_before_command_line_processing` site we'll add the switch in.
- PR #906 + #1035 (instrumentation that captured the trace).

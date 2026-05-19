# SPEC: Browser Pane HTTP Basic / Digest Auth

**Status:** Draft
**Date:** 2026-05-18
**Author:** AgentA
**Related:**
- `agentmux-cef/src/client/handlers.rs` (CEF `RequestHandler` host-side hook)
- `SPEC_MODAL_TRANSITIONS_2026_05_18.md` / modal-v2 chrome (the credential prompt UI)
- CEF API: [`CefRequestHandler::GetAuthCredentials`](https://cef-builds.spotifycdn.com/docs/stable.html?Doxygen/classCefRequestHandler.html#a)

---

## 0. TL;DR

Loading a site that returns `401 WWW-Authenticate: Basic ...` in the browser pane currently fails with `ERR_INVALID_AUTH_CREDENTIALS (-338)` because the host never implements CEF's `GetAuthCredentials` callback. CEF asks the embedder for credentials, the embedder is silent, the request fails.

Fix: wire `get_auth_credentials` on `AgentMuxRequestHandler` to surface a modal prompt (modal-v2 chrome via `TabModalLayer`), collect username + password from the user, and call the CEF callback. In-memory per-pane credential cache so a page that triggers multiple subresource auth challenges only prompts once.

Supports HTTP **Basic** and **Digest** auth (CEF handles both via the same callback). No proxy auth, no NTLM, no client-cert auth in v1.

---

## 1. Problem

User report 2026-05-18: opened `https://pulse.asaf.cc/` in the browser pane → page failed to load → host's error overlay showed `ERR_INVALID_AUTH_CREDENTIALS (-338)`. The site requires HTTP Basic auth.

CEF's `RequestHandler::get_auth_credentials` is the only hook by which the embedder can supply credentials in response to a `401`. Today our `AgentMuxRequestHandler` (`handlers.rs:316`) only implements `on_render_process_terminated`. The default behavior of an unimplemented `get_auth_credentials` is "no credentials available" → CEF treats the 401 as a fatal error.

---

## 2. Best practices

### Chrome's model
- Modal dialog at the top of the tab content with username + password fields, a "Remember me on this site" checkbox, OK / Cancel.
- Cached in the OS keyring (`chrome.passwords.sync`) when remembered; in-memory otherwise.
- Cancel → page loads the 401 response body (typically a server-provided "Auth required" page).

### Firefox / Safari
- Same shape. Slight UI differences. Same callback model.

### Cancel UX
- Important: a cancel must call `callback.cont(...)` with empty strings OR `callback.cancel()` so CEF doesn't hang waiting forever. CEF's API allows either path; we'll use `cancel()` to be explicit.

### Proxy vs. server auth
- CEF's same `get_auth_credentials` callback also fires for HTTP proxy auth (407). The `is_proxy` argument distinguishes. v1 spec covers server auth only; proxy auth out of scope (most users don't run through an authenticated proxy; if it becomes an ask, the same modal UI applies).

---

## 3. Architecture

### 3.1 Host side (Rust)

`AgentMuxRequestHandler::get_auth_credentials`:

```rust
fn get_auth_credentials(
    &self,
    browser: Option<&mut Browser>,
    origin_url: Option<&CefString>,
    is_proxy: bool,
    host: Option<&CefString>,
    port: i32,
    realm: Option<&CefString>,
    scheme: Option<&CefString>,
    callback: Option<AuthCallback>,
) -> bool {
    // Returning true tells CEF we'll provide credentials asynchronously
    // via the callback; returning false tells CEF to abort and surface
    // ERR_INVALID_AUTH_CREDENTIALS. Always return true so we can drive
    // the prompt UI.
    let Some(cb) = callback else { return false; };
    let block_id = /* resolve from browser */;
    let req = AuthRequest { origin: ..., realm: ..., is_proxy };

    // Send IPC event to renderer for that pane: "browser-pane-auth-required"
    // with { block_id, request_id, origin, realm, is_proxy }.
    // Store the callback in a registry keyed by request_id so the
    // renderer's reply can resolve it.
    register_auth_callback(request_id, cb);
    emit_event(...);
    true   // async — we'll callback.cont() / .cancel() later
}
```

Add a global registry in `agentmux-cef/src/browser_pane/auth.rs`:

```rust
static AUTH_CALLBACKS: Lazy<Mutex<HashMap<String, AuthCallback>>> = ...;
```

New IPC handlers (in `commands/`):

| Command | Args | Effect |
|---|---|---|
| `browser_pane_auth_submit` | `{ request_id, username, password }` | Look up callback, call `cb.cont(username, password)`, drop entry |
| `browser_pane_auth_cancel` | `{ request_id }` | Look up callback, call `cb.cancel()`, drop entry |

Timeouts: if the renderer doesn't reply within (say) 5 minutes, the host should drop the callback and cancel — preserves CEF state cleanliness. Implemented via a `tokio::time::sleep` task per registered callback.

### 3.2 Renderer side (TypeScript)

`BrowserViewModel` subscribes to `browser-pane-auth-required` (alongside the existing nav-state / title-change / favicon-urls listeners). On arrival:

1. Open a `tabModal.open({ kind: "browser-auth", ... })` modal request with the origin + realm.
2. User submits → call `RpcApi.BrowserPaneAuthSubmitCommand({ request_id, username, password })` (or the renderer-side `invokeCommand` equivalent).
3. User cancels → call the cancel command.
4. Modal closes.

New `TabModalRequest` variant in `frontend/app/tab/tab-modal.ts`:

```ts
export interface BrowserAuthRequest {
    kind: "browser-auth";
    blockId: string;
    requestId: string;
    origin: string;
    realm: string;
    isProxy: boolean;
    onSubmit: (username: string, password: string) => void;
    onCancel: () => void;
}
```

`TabModalLayer.tsx` dispatches this `kind` to a new `<BrowserAuthModalPanel>` component (`frontend/app/view/browser/components/BrowserAuthModal.tsx`).

### 3.3 UI

Modal-v2 chrome (header + body + footer per `internals/modal-system.md`). Body has:
- Title row: `"Authentication required"`
- Subtitle: `"<origin> says: <realm>"` (familiar to users from Chrome's prompt)
- Username input (autofocus)
- Password input (type=password)
- "Remember for this session" checkbox (deferred to v2; v1 always remembers per session)

Footer: Cancel + Sign in (Sign in is the green primary action; Enter inside the password field submits).

### 3.4 In-memory credential cache

Keyed by `${origin}#${realm}`. Per-pane (not global) — different panes might want different credentials for the same realm. Lives in the renderer's `BrowserViewModel` instance. Reset on pane close. Not persisted to disk in v1.

When CEF asks again for the same realm (subresource fetches, follow-ups), the renderer checks the cache and replies immediately without prompting. New realm → prompt.

### 3.5 Cancel path

User clicks Cancel:
1. Modal closes
2. `browser_pane_auth_cancel` IPC fires
3. Host calls `callback.cancel()`
4. CEF aborts the request → renders the 401 response body (typically server-rendered "Unauthorized" page) in the pane

That matches Chrome's behavior — the user gets a chance to see the server's error page.

### 3.6 Failure path (wrong password)

User enters wrong creds → CEF gets the 401 again → fires `get_auth_credentials` again → renderer prompts again. We can show a "Incorrect username or password" inline message in the modal on the second+ prompt for the same realm. Track per-realm attempt count.

---

## 4. Implementation order

### Phase α — Wire the callback + minimal prompt

1. `agentmux-cef/src/client/handlers.rs`: implement `get_auth_credentials`. Register the callback in a global mutex registry keyed by a generated request_id.
2. `agentmux-cef/src/browser_pane/auth.rs` (new): `register_auth_callback`, `resolve_auth_callback`, 5-min timeout.
3. `agentmux-cef/src/commands/`: `browser_pane_auth_submit` + `browser_pane_auth_cancel` IPC commands.
4. `frontend/app/tab/tab-modal.ts`: `BrowserAuthRequest` variant.
5. `frontend/app/tab/TabModalLayer.tsx`: dispatch case for `kind: "browser-auth"`.
6. `frontend/app/view/browser/components/BrowserAuthModal.tsx`: the modal panel (modal-v2 chrome).
7. `BrowserViewModel`: subscribe to `browser-pane-auth-required`, open modal, route submit/cancel.

### Phase β — Per-pane in-memory cache

8. Pane-scoped `Map<realmKey, {username, password}>` in `BrowserViewModel`.
9. When the IPC arrives, check cache before prompting; if hit, immediately reply with cached credentials.
10. On failed attempt (second `browser-pane-auth-required` for the same realmKey within ~10s), invalidate cache + re-prompt with "Incorrect password" inline.

### Phase γ — Polish (defer)

- Persisted credential store (OS keyring).
- Proxy auth (`is_proxy=true`).
- Client-cert auth (`OnCertificateError` + `SelectClientCertificate`).
- NTLM / Negotiate (`is_proxy=false` but mechanism != Basic/Digest — CEF mostly handles these internally on Windows via SSPI; verify before scoping).

---

## 5. Test plan

- [ ] `https://httpbin.org/basic-auth/user/pass` — prompt appears, submitting `user`/`pass` loads the success page.
- [ ] Wrong credentials → re-prompt with inline error.
- [ ] Cancel → server's 401 body renders (typically "Authorization required").
- [ ] Subresource auth on the same realm (page with images behind same auth) — only one prompt.
- [ ] Cross-realm subresources → two prompts.
- [ ] Two browser panes both pointed at `pulse.asaf.cc` — each prompts independently (no global cache).
- [ ] Close pane mid-prompt → no host-side memory leak, callback released within timeout.

---

## 6. Acceptance criteria

1. `ERR_INVALID_AUTH_CREDENTIALS (-338)` no longer surfaces for sites that return `401 WWW-Authenticate: Basic` — instead, the modal prompts.
2. Submit with correct credentials → page loads.
3. Cancel → 401 response body renders.
4. Per-pane cache: same realm asks once per pane per session.
5. Modal uses modal-v2 chrome — Cancel + Sign in footer, Enter submits.

---

## 7. Out of scope

- Persisted credential storage (OS keyring integration). v2.
- Proxy auth.
- Client certificates.
- "Remember me on this site" checkbox UI (v1 always remembers per session; v2 adds the toggle + persistence).
- Auto-population from a system password manager (1Password, etc.).

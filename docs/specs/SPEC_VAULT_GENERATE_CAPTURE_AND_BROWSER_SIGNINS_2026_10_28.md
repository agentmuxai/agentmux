# SPEC: secrets nobody types. The Vault generates, captures, delivers and rotates them, and the browser saves, generates and fills sign-ins

**Status:** Approved 2026-10-29. The operator's words, relayed by AgentY: "yes, password spec looks good, proceed". The recommended answers in §12 stand. Spec task #6760; build #6761, one PR per phase in §11's order.
**Author:** Agent2-6 (Vault). **Browser side reviewed by:** Korp, 2026-10-29 (§13); corrections folded in.
**Verified on:** `main` at `38d929fe6` (2026-10-28).
**Extends:** `SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md` (the Vault spec, "the Vault spec" below), and folds in its P8 (#133, "agents typing secrets they hold, or filling saved passwords").

## 1. Summary

The operator asked on 2026-10-28 whether the browser can save passwords (it can't), then said: "yes, can we also use google's generate safe password when creating accounts?", and then asked whether saved sign-ins are bound to browser profiles. This spec covers the whole path of a secret that no person should have to type:

- **A. The Vault makes and moves secrets itself.** `generate` creates a strong value inside srv. `capture` stores a value a command prints, without the agent seeing it. Values are delivered to a command's environment (built), its stdin, or a file only it reads. `rotate` replaces a value and keeps the old one for a while.
- **B. The browser saves sign-ins.** When a person signs in, the pane offers "Save to Vault". On a later visit it fills the sign-in only after the person approves in a window outside the page.
- **C. The browser generates passwords on sign-up forms.** AgentMux, not Google, makes the password, fills both fields and stores it in the Vault *before* the form is sent.
- **D. Agents may create accounts.** On a sign-up the person asked for, an agent may trigger C, never seeing the value, with a per-site approval, a visible countdown and an audit line.
- **E. Import from Chrome and Google Password Manager**, as a later step.
- **F. Each saved sign-in belongs to one browser profile** (section 9; decided by the operator on 2026-10-28).

The invariant is the Vault spec's, unchanged: **an agent directs a value; it never holds one.** No new route returns a value to an agent.

## 2. What exists today (from the code)

| Area | Today | Where |
|---|---|---|
| Vault store | Entries sealed one by one with ChaCha20-Poly1305 (entry id as associated data), one 256-bit Vault key in the OS keychain; tables for entries, activity, grants and `db_browser_signins` | `crates/srv/src/backend/storage/vault.rs:18-66` |
| Kinds | 13, including `api-key`, `login` (parts `password`, `totp`) and `note` | `crates/srv/src/backend/vault/kinds.rs:38-58` |
| Agent tool | `Vault` `list` (no values) and `request` (asks the person to add an entry) only | `crates/mcp/src/tools/vault_tool.rs:16-56` |
| Commands | `Terminal` `run` with `secrets: {ENV: id}`; output scrubbed of each value and its base64, URL and hex forms; first use per entry and agent asks | `crates/srv/src/server/shell_secrets.rs`, `backend/vault/scrub.rs` |
| Browser fill | `Browser` `fill` with `vault`: host types the value, the field is tainted (`data-agentmux-vault`), per-site grants | `crates/cef/src/browser_api/vault_fill.rs`, `crates/srv/src/server/browser_vault_fill.rs` |
| Password refusal | `password_like()` refuses `type=password`, `autocomplete=one-time-code` and fields named like a password or code, for every agent fill | `crates/cef/src/browser_api/vault_slots.rs:88-103`, `browser_vault_fill.rs:85-101` |
| Randomness | `ring::rand::SystemRandom`; the only generator is `suggest_passphrase()` for `.vault` export (8 words, 64 bits) | `crates/srv/src/backend/vault/transfer.rs:78-85` |
| HTTP sign-ins | HTTP Basic/Digest credentials in the OS keychain, keyed `browser-auth:{identity_id}:{sha256(origin, realm, proxy)}`; the Vault only notes them (masked username) in `db_browser_signins` | `crates/srv/src/identity/browser_credential_store.rs`, `server/service/credential.rs` |
| Approval outside the page | `CredentialApprovalWindow` ("Use saved sign-in?") is a separate top-level CEF window with no `data-blockid`, so no agent's `UI` click can reach its button | `frontend/app/view/credential-approval/CredentialApprovalWindow.tsx:1-25` |
| Profiles | Personal (the global jar, can't be deleted), named profiles (`profile:<id>`, a disk jar at `<cef-cache>/profile-<id>`, "Agents may use it" switch), Incognito (`incognito:<jar>`, in memory, gone when the tab closes) | `backend/browser_profiles_store.rs`, `server/browser_identity.rs`, `crates/cef/src/browser_pane/identity.rs` |
| Chromium password manager | No UI path. Browser panes are created Alloy-style (`browser_pane/creation.rs:225`, `creation_views.rs:134`, `floating_pane.rs`), and the save bubble and autofill popup are Chrome-style UI; only the DevTools popup is Chrome-style (`app/mod.rs:440`). On narko, the 339 dev cef-cache profiles each have `Login Data` and `Web Data`, which Chromium creates at profile start, and every one holds 0 `logins` rows and 0 autofill rows, after many sign-ins. No code sets `credentials_enable_service` or `profile.password_manager_enabled`. Not yet checked live with a real form submit. B2 adds the prefs and a ceftest (§5.1) | Korp's review, 2026-10-29 |
| Form noticing | No form-submit hook today. An isolated-world injector with a CDP binding already exists for the mouse side buttons | `browser_pane/side_button_cdp.rs` |

Two facts shape the design:
1. **Today's HTTP sign-ins are keyed by the pane's agent account (`identity_id`, from the block's active agent instance), not by its browser profile.** A tab the person opens without an agent resolves to `identity_id: ""` and saves nothing reusable. Section 9 moves them onto profiles.
2. **The Vault spec answered "store passwords, don't fill them yet" (its §13 question 2).** The operator's yes on 2026-10-28 is that later spec. Its §13 question 2 and P8 row get amended in the build PR to point here.

## 3. Goals and non-goals

**Goals**
- No person types, reads or pastes a password, key or token that AgentMux can generate, capture or fill.
- No agent ever holds a secret value, in its context, transcript, tool results, screenshots or the messages it sends.
- A generated password can't be lost: it is in the Vault before any site sees it.
- Every save, fill, generation, capture and rotation is in the Vault's Activity (and, per the Vault spec §5.6, Warden's Secrets section), with who, where and when, never the value.

**Non-goals**
- Google's own suggested password. Chrome's password manager isn't available to CEF, and it would store the password in Google's account rather than the Vault.
- Passkeys (WebAuthn). A separate spec: they don't fit "fill a field".
- Syncing the Vault between devices.
- Agent-typed secrets (Vault spec §13 question 1 stays "no").

## 4. A. The Vault makes and moves secrets itself

### 4.1 `Vault` `generate`

`Vault` `generate` `{kind, label, why, rules?, site?, username?}` creates an entry whose main value srv makes, and returns only its id. It takes kinds `api-key`, `login` (the password part) and `note`.

The generator:
- uses `ring::rand::SystemRandom` with rejection sampling, so every character is equally likely (no modulo bias);
- defaults to 20 characters from upper and lower case letters, digits and `-_.!@#%^*`, with at least one of each class, and no look-alike characters (`0O1lI`) so a person can read it if they ever must;
- takes `rules` in the `passwordrules` format (`minlength`, `maxlength`, `required`, `allowed`, `max-consecutive`), which a site's own `passwordrules` attribute (§6.2) also supplies;
- refuses rules that leave less than 80 bits of entropy, and says how many bits they leave;
- adds a `wordlist` style (6 words from the existing `WORDS` list, 48 bits; suggested for things a person must type on a TV or phone), only when asked.

Creating an entry this way doesn't ask the person: it makes a value nobody has seen, which holds no secret of theirs yet. It is logged ("generated by Agent2-6, for: <why>").

### 4.2 `Vault` `capture`: a value a command prints goes straight in

Many keys are born in a command's output: `gh auth token`, `aws iam create-access-key`, a cloud console's CLI, `openssl rand`. Today the agent sees them.

`Terminal` `run` gains `capture: [{as: {kind, label, part?}, from: <rule>}]`. A rule is one of:
- `{"stdout": "all"}`: the whole output, trimmed;
- `{"json": "<path>"}`: a field of JSON output (`$.AccessKey.SecretAccessKey`);
- `{"regex": "<pattern>"}` with one capture group;
- `{"line": n}`.

srv applies the rules to the raw output **before** anything is published, stores each match as a new entry (or a new version of an entry named by id, §4.4), and adds the captured values to the job's scrub list. So the agent's output, the conversation row and the job log all show `[vault:<id>]` where the value was. Then it returns the ids.

Details:
- **No match** fails the capture, and the output is withheld from the agent, whole, because srv can't tell which line held the secret. The agent gets the exit code, the line count and the reason ("the JSON path matched nothing"). The person can open the full output in the Vault, behind the OS confirmation that Reveal uses.
- A captured value shorter than 8 characters is refused, since the output couldn't be cleaned of it reliably.
- Capture works with or without `secrets`. The command is approved the way §7.3 of the Vault spec approves a command given secrets, so the person sees the command line once per agent and command. "Always" covers only that command line.

### 4.3 Delivering a value: environment, stdin, file

`Terminal` `run` already takes `secrets` as environment variables. It gains two more targets:
- **`stdin_secret: "<id>[.part]"`**: the value, then a newline, is written to the process's stdin, which is then closed. For tools that read a password from stdin (`docker login --password-stdin`, `gh auth login --with-token`).
- **`secret_files: {"<ENV_NAME>": "<id>[.part]"}`**: srv writes the value to a file in a fresh private folder (owner-only on Unix; on Windows an ACL for the user alone), puts the file's path in `ENV_NAME`, and deletes the folder when the process exits, or at srv's next start if it crashed. For tools that only read a key file (`--key-file`, `GOOGLE_APPLICATION_CREDENTIALS`, an SSH key).

Both are scrubbed and approved exactly as `secrets` is. A value never goes in an argument (it would be in `ps` and shell history). WSL and SSH follow the Vault spec §7.4 proposal: WSL through `WSLENV` for environment variables, and stdin when it is built; files stay host-only, since a file in a remote home is worse than an environment variable.

**HTTP headers are not a target** in this spec. An agent that needs an API call with a key runs `curl -H "Authorization: Bearer $KEY"` with `secrets`, as today. A dedicated "fetch with a Vault header" action would only duplicate it.

### 4.4 Rotation and versions

An entry keeps versions. A new value (by `generate`, `capture`, an edit or a browser save) becomes the current version. The previous one is kept for 7 days, marked "replaced on …", restorable from the Vault pane, and then deleted. Only the current version is filled or delivered.

`Vault` `rotate` `{id, set: <command>, verify?: <command>}`:
1. generates a new value under the entry's rules, as a **pending** version that nothing is given yet;
2. runs `set` with the pending value (any target from §4.3, by default `NEW_SECRET`) and the current value in `OLD_SECRET`, for services that need the old one to change it;
3. runs `verify`, if given, with the pending value;
4. on success makes the pending version current; on any failure keeps the old one current and the pending one for 7 days, marked "rotation failed at step 2".

If `set` succeeded but `verify` failed, the service may already hold the new value. So the pending version is kept, the Vault pane shows both, and the agent is told which step failed. Nothing is thrown away.

`rotate` is approved as one command line per agent. The prompt names `set` and `verify`, and says the entry's value at the service will change.

Captured keys (`aws iam create-access-key`) usually rotate by creating a new key and deleting the old. That is `rotate` with `set` creating it, a `capture` rule naming the entry, and a second `run` with the old version (`id@previous`) that deletes it.

## 5. B. The browser saves sign-ins and fills them on approval

### 5.1 Noticing a sign-in

The host injects a small script, in an isolated world, into every frame of every tab. It notices a form, or a set of fields without a form, that holds one password field (`type=password`) and is about to be sent, by a submit event, a click on its submit button or Enter in one of its fields. A form with two or three password fields, or with `autocomplete="new-password"` on its password field, is a sign-up or password change (§6), not a sign-in.

It reads the username (the field marked `autocomplete="username"` or `email`, else the last text or email field before the password) and the password, and hands both to the host through a binding that exists only in its isolated world. So the page's own scripts can't call it to plant a value. srv holds the pair in memory for 2 minutes, keyed by tab, frame origin and form, and never logs either.

**How it is built (Korp's review):**
- **Reuse the side-button injector** (`browser_pane/side_button_cdp.rs`), not a second one. For each frame: `Page.createIsolatedWorld{worldName}`, then `Runtime.addBinding{name, executionContextName: <world>}`, then `Runtime.evaluate` in that context. Calls arrive as `Runtime.bindingCalled` in `note_event`.
  - Use **one world per frame**, shared with the side-button script: two bindings in it, or one binding carrying both. Two worlds per frame doubles the cost per document.
  - Cross-process iframes are separate CDP targets. `attach_frames` and auto-attach cover them, re-run on `Target.attachedToTarget`, and `on_thaw` re-installs after a hibernation discard. The injected set is keyed by frame id and loader id.
  - **Never `Page.addScriptToEvaluateOnNewDocument`.** It runs only while the Page domain is on, and an enabled Page domain takes the page's JS dialogs away from the host's handler.
- **The frame's origin comes from the host, never from the payload.** `bindingCalled` carries an `executionContextId`, not a frame or an origin. The host keeps a map from context id to session, frame id and origin, built from `createIsolatedWorld` and the frame tree (`frames::FrameRef` already holds origin and session). Opaque origins are refused: `about:blank`, `srcdoc`, sandboxed `null` and `data:`.
- **The password stays out of the event stream.** `note_event` runs `serde_json::from_slice` on every `bindingCalled` message, which would leave the password in a plain `String`. The sign-in binding's payload takes a dedicated zeroizing parse path, is forwarded, then dropped. Tests check two things: no CDP trace or log line contains a `bindingCalled` payload, and no agent-facing raw-CDP route can see this binding.
- **Only trusted events.** The listeners check `event.isTrusted`, which drops a page's own `form.dispatchEvent(new Event('submit'))` and `el.click()`. A page can still trigger real offers by luring clicks, so offers are rate-limited per origin, one shown at a time.
- **Identifier-first sign-ins** (Google, Microsoft and others ask for the username, then show the password field later in the same document): the last username-only submit is held for 2 minutes per tab and origin, zeroized, and joined to the password step. On a page that asks only for the username, the "Sign in as" menu (§5.3) fills the username with the same approval; filling the password at the later step is v2.
- **Chromium's own manager stays off**, belt and braces. B2 sets `credentials_enable_service=false`, `profile.password_manager_enabled=false`, `autofill.profile_enabled=false` and `autofill.credit_card_enabled=false` through the PreferenceManager of every request context, Incognito included, as `browser_panes/autocomplete.rs` already does for `search.suggest_enabled`. A ceftest then submits a localhost sign-in form in Personal, a named profile and Incognito, and asserts no bubble and 0 rows in `Login Data` and `Web Data`.
- **Incognito** gets the script, since "Fill from Personal" and sign-up generation need detection, but never offers Save (§9.1).

**Limits** (noticing is best-effort):
- **A page can cancel a submit before the script sees it.** The script is added after the document commits, a few ms in. A capture-phase listener the page registered earlier can run first and call `stopImmediatePropagation()` on the submit. That is why the script also listens for the click and for Enter.
- **Busy pages.** At most 9 frames per tab get scripts (`MAX_FRAMES`). On a page with more, cross-process iframes get none, so a sign-in inside such an iframe isn't noticed.
- **Cost.** About 1% of a core with 15 cross-site iframes, measured on Linux/X11 only. Windows and macOS use the same CDP client but are not measured.

### 5.2 "Save to Vault"

After the page navigates or the form reports success (a navigation, or the password field leaves the page), the pane shows a bar in its chrome, not in the page: **"Save the sign-in for example.com (us***@mail.com) to the Vault? [Save] [Never for this site] [Not now]"**.
- **Where:** an in-flow band between the nav bar and the page, in the same stack as the Auto-approve confirmation, the BrowserAttention banner (`browser-view.tsx`, about line 715) and the driven-by strip. One band shows at a time; the attention and bypass banners come before the save bar. The page area shrinks by the band's height while it shows, as it does for those banners. At the 250 px minimum pane width, the question takes the first line and the buttons the second.
- **Only host-supplied text.** The bar's sentence holds only the frame's exact origin, from the host, and the masked username, from srv. It never holds the page's `<title>` or any other page text, so a page can't make the question say something else.
- **Save** stores a `login` entry: site (exact origin), username, password, the profile (§9) and a label made from the host name. If the profile already holds that site and username with another password, it says "Update the saved password?" instead, and the old one is kept as a previous version (§4.4).
- **Never for this site** is remembered per profile and site.
- The bar goes after 2 minutes or the next navigation away, and the held pair with it.

On a tab an agent drives, the person may type a password during a handoff. Then the bar is the same and goes to the person. The agent's tool results say only "the person was offered to save a sign-in".

### 5.3 Filling a saved sign-in: approve outside the page

On a page whose frame holds a sign-in form for an origin the profile has a login for, the pane chrome shows a **key button with the number of saved sign-ins** for that page, in `.browser-nav-group--profile` next to the profile button, since sign-ins are per profile. Its label is only in the tooltip; it doesn't go in the address row, which already stacks below 388 px.
- **The button opens a native menu, not a DOM one** (`ContextMenuModel.showContextMenu`, through `getApi().showContextMenu`), listing "Sign in as us***@mail.com" for each login. This matters for approval. `UI` `click` synthesizes a real mouse click in its caller's pane subtree (`ui_handlers.rs:339`). It is refused on an agent-driven tab, but on a tab the person opened, an agent could click DOM chrome, and that click is indistinguishable from the person's. A native menu is outside the DOM, so no `UI` click reaches its items, and an item being chosen really is the person's approval. The button itself only opens the menu.
- **The person chooses an item:** the host fills the username and password into that frame's fields, the same way `fill` types (`Input.insertText`, with the setter fallback), taints both and records it.
- **Checks before typing (shared with `generate_password`):**
  - Run vault_fill's guard (`expect_origin` and `expect_fingerprint`, with `check_still_approved` before and after focus) on both fields.
  - Re-check the password field after typing the username, since a page can swap it then.
  - Run the visibility check of §10.2 on both fields.
- **An agent asks** with `Browser` `signin` `{pane, entry?}`: the host opens the credential-approval window that already exists for HTTP sign-ins, a separate top-level window no agent can click. It shows "**Agent2-6** wants to sign in to **example.com** as **us***@mail.com** in its tab. [Allow once] [Always for Agent2-6 on this site] [Deny]". On allow, the host fills. The agent gets `{filled: true, entry: <id>}` and never the values. "Always" is a grant per agent, site and profile, listed in the Vault and Warden and revocable there. A fill under a grant shows a 5-second "Agent2-6 is signing in to example.com [Stop]" notice in the pane chrome before the host types.

Rules on where a sign-in may go:
- **Exact origin only.** A login saved for `https://accounts.example.com` fills only a frame whose origin is exactly that: not `http`, not another subdomain, not a look-alike. The person can add another origin to an entry in the Vault ("also use on …").
- The target fields must be a `type=password` field and its username field in the same form or field group, in that frame. A login is never typed into a text field, so `password_like()` keeps refusing generic `fill` into passwords, and `signin` is the only way in.
- No fill into a page loaded over plain `http` (other than `localhost`), or a pane with a certificate error. *To settle in B2:* how the host knows a pane has a certificate error today (the request handler's `on_certificate_error`, or the CDP Security domain). Korp hasn't checked which, so B2 starts by finding out.
- A one-time code (`login.totp`) is not filled. That is §12 question 5.

### 5.4 HTTP sign-ins join the Vault

HTTP Basic/Digest credentials (`browser_credential_store`) become `login` entries with a realm, in the same profile-keyed store (§9.3). The "Use saved sign-in?" window keeps working as now. The keychain copies are migrated once and then deleted. One store means one list, one set of grants and one Activity.

## 6. C. Generate a strong password on a sign-up form

### 6.1 Noticing a new-password field

The script of §5.1 also marks a **new-password field**:
- a `type=password` field with `autocomplete="new-password"`. This requires `type=password`, since sites put `new-password` on ordinary fields to stop autofill, the case `secret.rs:91-94` already handles; or
- two `type=password` fields in one form where the second's label, name or `autocomplete` says confirm, repeat or retype; or
- a form with a password field and a submit button labelled like "Create account", "Sign up", "Register" or "Join", with no `current-password` field.

A form that also holds a `current-password` field is a **password change** (§7.3).

### 6.2 "Use a strong password"

When the person focuses a new-password field, the key button (§5.3) shows that it has an offer. Its native menu holds **"Use a strong password (saved to the Vault)"**, so, as with sign-in, no agent's `UI` click can choose it for the person. When the person chooses it:
1. srv generates a password (§4.1) under the site's rules: the field's `passwordrules` attribute, else its `minlength` and `maxlength`, else the defaults, clipped to `maxlength`.
2. srv stores it as a `login` entry for the frame's origin, in the tab's profile (§9), with the username field's current value (empty if none yet), marked **"unconfirmed: made on sign-up, not yet sent"**. Storage happens before any field is filled, so the value exists only in the Vault when the site first sees it.
3. The host fills the new-password field and its confirm field, and taints both.
4. When the form is sent (§5.1), srv records the username as it was sent and marks the entry "confirmed". A form not sent within 30 minutes leaves the entry "unconfirmed" in the Vault, where the person can keep or delete it.

The person never sees the password unless they reveal it in the Vault. To type it elsewhere (a phone, a TV), they reveal it there, behind the OS confirmation, as for any entry.

If the site rejects the password (it shows an error next to the field and the form isn't sent), the person or agent clicks the offer again. A new version replaces the unconfirmed value, and the rejected one is kept as a previous version.

## 7. D. Agents create accounts

### 7.1 The action

`Browser` `generate_password` `{pane, ref, username?}` does §6.2 for an agent, on a new-password field it found by `snapshot` or `find`. The agent gets `{entry: <id>, filled: ["e12", "e13"], rules: "20 chars, from the field's passwordrules"}` and never the value. It then fills the other fields (name, email) itself and clicks the submit button, which, being a committing click, asks the person as today (agent-driven panes spec §5.4) unless a grant covers it.

This replaces Vault P8 / #133: agents don't type secrets they hold, and they fill saved passwords only through `signin` (§5.3).

### 7.2 Guardrails, and why these and not "only the person"

The operator's rule (2026-10-25) is that agents may do anything except speak in the operator's voice, and that sensitive actions get a visible countdown with a veto rather than an "operator only" gate. Creating an account isn't the operator's voice, so an agent may do it. The guardrails make it visible and reversible:
- **First use per agent and site asks**, in the credential-approval window: "**Agent2-6** wants to create an account on **example.com** with a password the Vault makes. [Allow once] [Always for Agent2-6 on this site] [Deny]". With "Always" (or a standing browser grant that includes the `accounts` switch, superpowers report B13), later sign-ups on that site only show the countdown.
- **A 10-second countdown with Stop** in the pane chrome before the host fills: "Agent2-6 is creating an account on example.com: password from the Vault [Stop]". Stop deletes the unconfirmed entry and fills nothing.
- **The audit line** in the Vault and Warden: agent, site, profile, entry, time, and whether the form was sent.
- **The profile rule (§9):** an agent creates accounts only in its own private or Incognito tab, or in a profile the person has switched to "Agents may use it". The entry goes to that profile.

### 7.3 Password changes need the person

On a form with a `current-password` field (a password change), `generate_password` asks the person every time, in the approval window, and "Always" is not offered. That is the one place where an agent's mistake costs the person their own account: the site's old password stops working. The old value is kept as the previous version (§4.4), so a change the person approves by mistake can still be undone at the site. This is a new guard. It is kept because a rotated personal password locks the person out, which the countdown alone can't undo once the form is sent. It doesn't stop a forged operator voice, so it is flagged here for the operator's call (§12 question 3).

## 8. E. Import from Chrome and Google Password Manager (later)

Chrome and passwords.google.com export a CSV with `name,url,username,password,note`. It joins the Vault spec's P5 importers: one parser, the same preview, the plaintext-file warning, and the same fuzzing. Each row becomes a `login` entry with its origin taken from `url`, and the person picks the profile for the whole import (default Personal). Rows whose `url` is an Android app (`android://…`) are kept as entries without a site, since there is nothing to fill them into. After the import, the person is reminded to delete the CSV and, if they want, the passwords in Google's manager.

## 9. F. Every saved sign-in belongs to one browser profile

The operator asked: "right, but are they bound to browser profiles?" **Decided on 2026-10-28:** yes, as recommended. AgentY relayed the operator's words: "right, lets do that". Each sign-in is bound to one profile and offered only there. Filling needs approval. Incognito never saves. Moving a sign-in between profiles is an explicit action. §9.1 and §9.3 describe that decision. §9.2 (a password made in Incognito) and §9.4 (profile deletion) are details this spec adds, and they are still open as §12 questions 4 and 6.

### 9.1 How it fits the profile and cookie-jar model

A tab's profile is its block's `browser:identity`, set when it is opened and never changed:
- absent means **Personal**, the global jar;
- `profile:<id>` is a **named profile**, a disk jar at `<cef-cache>/profile-<id>`;
- `incognito:<jar>` is **Incognito**, an in-memory jar dropped when no tab uses it.

A saved sign-in is the password half of what a jar's cookies are the session half of, so it follows the same key:
- Each `login` entry with a site carries `profile`: `personal` or `profile:<id>`. A sign-in is offered and filled only in tabs of that profile.
- **Incognito never saves** a sign-in and never offers to. It can offer "Fill from Personal" for a site Personal holds, and that asks every time, in the approval window, with no "Always". An agent's Incognito tab follows the same rule.
- **An agent's tab on a named profile** (allowed only when the person has switched that profile to "Agents may use it") gets that profile's sign-ins only, through `signin` and its approval (§5.3). An agent's private tab (the default for `Browser` `open`) gets none.
- **Moving or copying** a sign-in to another profile is an explicit action in the Vault pane ("Move to profile…", "Copy to profile…"), logged. Nothing moves by itself.
- **Logins without a site** (`login` entries the person typed into the Vault, or imported Android rows) belong to no profile. They are never offered in a tab, only by explicit choice in the Vault.

### 9.2 A password made in Incognito

§6 and §7 must store a generated password, or it is lost, but Incognito never saves. **Recommended:** a password generated in an Incognito tab is stored in **Personal**, tagged "made in an Incognito tab", and the person is told in the bar. Incognito keeps no cookies and remembers nothing, but the account it created is real and needs its password. See §12 question 4.

### 9.3 Today's HTTP sign-ins

They are keyed by the agent account (`identity_id`) of the pane's agent instance (§2, fact 1). The migration of §5.4 assigns each existing credential to **Personal**, since no record says which profile it was used in, and lists them in the Vault as "moved from the old sign-in store: check the profile". After that, HTTP sign-ins are keyed by profile like form sign-ins. Agent account identity stops being part of the key.

### 9.4 When a profile is deleted

Today deleting a profile closes its tabs and drops its jar folder, and the menu says "its saved sign-ins go" (`browser-profiles.ts:60`). That is fine for cookies, which a sign-in recreates, but not for passwords, which may exist nowhere else. **Recommended:** the delete confirmation says how many saved sign-ins the profile holds and offers **[Move them to Personal]** (the default) or **[Delete them too]**. Moved entries are tagged "from deleted profile <name>". Profile ids are never reused, so an orphan entry can't attach to a new profile with the same name. The deletion and the choice are logged in Activity.

Profile changes are today refused to agents (`operator_only`, `app_api/browser_profiles.rs:72`). That predates the 2026-10-25 rule and isn't changed by this spec.

## 10. Security

### 10.1 What an agent can and can't reach

| Path | Result |
|---|---|
| Tool results of `generate`, `capture`, `rotate`, `signin`, `generate_password` | Ids and outcomes only |
| `snapshot`, `find`, read-back | Password and tainted fields show `[secret]` (exists) |
| Screenshots (`UI` `screenshot`, the browser's) | Secret and tainted fields blacked out (exists) |
| `eval` on its own driven pane | Refused (exists) |
| `eval` on a tab the person opened, after a fill | **New:** refused while a tainted field exists in the tab, until it navigates; read-only script results are scrubbed of the filled values as command output is |
| Command output after `capture` or `secrets` | Scrubbed (`[vault:<id>]`); on a failed capture, withheld |
| The approval window | A separate top-level window with no `data-blockid`; no `UI` click reaches it (exists) |
| "Sign in as …" and "Use a strong password" | Items of a native menu, out of the DOM, so no `UI` click reaches them; an agent uses `signin` and `generate_password`, which ask in the approval window (Korp's review) |
| The save bar | DOM chrome. An agent's `UI` click on [Save] is allowed, since saving takes nothing out of the Vault; it is logged as the agent's |

### 10.2 What a page can do

- **Read what it was given.** A site's own scripts can read the password typed into its own field, as with every password manager. Exact-origin filling (§5.3) is what keeps a password away from other sites.
- **Fake a sign-in to plant an entry.** It could only offer to save its own fields for its own origin, and only the person (or an agent, visibly) saves. It gets nothing out.
- **Hide a field to harvest a fill.** The host fills only visible, enabled fields that the person or agent targeted. Today's `fill` rejects only a 0×0 rect (`act.rs:135`). `signin` and `generate_password` share one stricter check. A field must pass all of these:
  - its computed `visibility`, `opacity` and clip;
  - it is on screen;
  - no ancestor has `display:none` or `aria-hidden`;
  - `elementFromPoint` at the field's centre, inside its frame, returns the field itself.
- **Draw over the offers.** It can't: the bar, the band and the native menus are outside the page's native rect. HTML fullscreen covers the chrome, so offers just aren't visible then.
- **Call the capture binding.** It exists only in the isolated world.

### 10.3 Logs and messages

No value in any log, Activity row, transcript, MuxBus message, PR or crash report. srv's held sign-in pair (§5.1) and the generator's buffers are `Zeroizing` and dropped after use. The redaction layer (`crates/common/src/redact.rs`) is the backstop, not the mechanism.

**Hibernation.** The held pair (2 minutes) normally expires before the earliest freeze (5 minutes hidden) or sleep (30 minutes). It is still dropped explicitly, keyed by block id, on pane close, freeze, sleep, discard and profile change. A page with a changed form field, a typed password included, is kept awake by `browser_pane_hibernation_check`, so a half-typed sign-in isn't discarded under the person. After a discard, `on_thaw` re-installs the script. No held pair survives a thaw, and the person is told so if a save bar was showing.

## 11. Phases

| Phase | What | Needs |
|---|---|---|
| **V1** | `Vault` `generate`; `stdin_secret` and `secret_files` on `Terminal` `run` | Vault spec P7 (built) |
| **X1** | Expiry dates and reminders (§15.2) | V1 |
| **V2** | `capture` on `Terminal` `run`; versions (§4.4), which capture into an existing entry is the first to need | V1 |
| **A1** | Capture from a browser pane: a field or text shown once, and a download (§15.1) | V2 |
| **V3** | `Vault` `rotate` | V1, V2 |
| **B1** | Profile on every sign-in; HTTP sign-ins migrated into the Vault (§5.4, §9.3); profile delete offers Move/Delete (§9.4) | — |
| **B2** | Notice sign-ins, "Save to Vault", fill by the person's click (§5.1-5.3) | B1 |
| **B3** | `Browser` `signin` with the approval window and grants (§5.3) | B2 |
| **C1** | "Use a strong password" on sign-up (§6) | V1, B1 |
| **D1** | `Browser` `generate_password`, countdown, password-change rule (§7) | C1, B3 |
| **E1** | Chrome / Google Password Manager CSV import (§8) | Vault spec P5 |

V1 and B1 can be built in parallel. Korp reviews A1, B2, B3, C1 and D1. Versions moved from V1 to V2 when V1 was built: nothing in V1 replaces a value, so V2 is where they are first used. Separately from these phases, the Vault request form being cut off before it opens (the #6761 card's item d) is a bug fix with a PR of its own.

## 12. Decisions (operator, 2026-10-29: the recommended answers stand)

Answered: 1 yes; 2 yes; 3 yes; 4 yes; 5 not now; 6 yes; 7 yes, as the last phase; 8 seven days. The questions as they were put:

1. **Fill approval.** Does the person's own click on "Sign in as …" in their own tab count as approval, with an agent's `signin` asking in the window with "Always for this agent on this site"? *Recommended: yes,* the same shape as the Vault's form and command grants.
2. **Agents creating accounts (D).** Allowed, with the first use per agent and site asking, a 10-second countdown with Stop, and the audit line? *Recommended: yes.*
3. **Password changes (§7.3).** An agent changing an existing account's password asks the person every time, with no "Always"? *Recommended: yes.* This is a guard beyond the countdown, kept because a wrong change locks the person out of their own account.
4. **A password made in Incognito (§9.2).** Store it in Personal, tagged? *Recommended: yes;* otherwise it is lost.
5. **One-time codes.** Should the Vault fill a TOTP code from a login's `totp` part, with the same approval as `signin`? *Recommended: not now;* the second factor is worth keeping on the person's device until sign-in filling has run for a while.
6. **Profile delete (§9.4).** Move a deleted profile's sign-ins to Personal by default? *Recommended: yes.*
7. **Chrome import (E).** Build it after B2? *Recommended: yes, as the last phase.*
8. **Rotation's old versions.** Keep the previous value 7 days? *Recommended: 7 days;* long enough to notice a failed change, short enough not to keep dead secrets.

## 13. Browser-side review (Korp, 2026-10-29, on `origin/main` `2a368f171`)

Verdict: B2 to D1 are buildable as written, with these corrections, all now in this spec:
- **Chromium's password manager:** no UI path today, since panes are Alloy-style and 339 profiles show 0 saved rows. A belt (prefs off) and a ceftest go in B2 (§2, §5.1).
- **Noticing forms:** reuse `side_button_cdp.rs`, one isolated world per frame, never `addScriptToEvaluateOnNewDocument`, `isTrusted` only, offers rate-limited, and the stated limits (§5.1).
- **Holes fixed:**
  1. The frame origin comes from the host's context map, never the payload, and opaque origins are refused (§5.1).
  2. The binding's payload takes a zeroizing parse path, never the generic `note_event` JSON (§5.1).
  3. Identifier-first sign-ins (§5.1).
  4. An agent's `UI` click on DOM chrome could pass for the person's approval, so sign-in and generation offers are native menus (§5.3, §6.2, §10.1).
- **Placement:** the save bar is a band below the nav bar; the key button sits next to the profile button (§5.2, §5.3). Only host text goes in their sentences.
- **Fill checks:** a shared visibility check; vault_fill's origin and fingerprint guard on both fields; the password field is re-checked after the username (§5.3, §10.2).
- **Hibernation:** the held pair is dropped explicitly, and none survives a thaw (§10.3).

Still open, for B2:
- How the host knows about a certificate error (§5.3).
- A live probe of the password-manager ceftest. Korp offered to run it in a task dev build.

## 14. Tests

- **Generator:** every length and rules combination yields the required classes; chi-square over 10⁶ characters shows no bias; refusal below 80 bits; `passwordrules` parser fuzzed.
- **Capture:** each rule kind; no match withholds output; the value never appears in the job log, conversation row, `wait` output or Activity; base64, URL and hex forms scrubbed.
- **Delivery:** `stdin_secret` closes stdin; `secret_files` folder is owner-only and gone after exit and after a crash plus restart.
- **Rotation:** success, `set` failure, `verify` failure (pending kept), restore of a previous version.
- **Browser (CEF integration tests):** no password-manager bubble and 0 `Login Data`/`Web Data` rows after a localhost sign-in in Personal, a named profile and Incognito; `typeof window.<binding>` is `undefined` in the page's main world; no CDP trace or log line holds a `bindingCalled` payload; untrusted `dispatchEvent` submits and `el.click()` are ignored; opaque-origin frames refused; identifier-first sign-in joins the username step to the password step; the sign-in and generation menus are native (a `UI` click can't choose them); sign-in noticed by submit event, button click and Enter; not by a page calling the binding; save bar outside the page; fill only on exact origin and visible `type=password`; refused over http and certificate errors; sign-up detection on the three patterns, with no false positives on `new-password` text fields; entry stored before the fill; unconfirmed to confirmed on submit; Incognito saves nothing and fills Personal only with approval; an agent's `UI` click can't reach the approval window; the countdown's Stop deletes the entry.
- **Profiles:** sign-ins offered only in their profile; migration to Personal; profile delete with Move and with Delete.
- **No value anywhere:** a test agent runs every action and its transcript, tool results and screenshots are searched for the values (and their encodings).

## 15. Addendum, 2026-10-29: capture from a browser pane, expiry dates, and recipes

The build card for #6761 also asked for three things this spec had left out: (a) saving a secret a web page shows once, (b) delivering values to the usual targets, and (c) expiry dates with reminders. The operator approved (a) and (c) on 2026-10-29. AgentY relayed the operator's words: "yes, fix those too". The design below is the recommended one. (b) needs no new action: §15.3 gives the recipes.

### 15.1 A1: capture a secret a web page shows once

GitHub (an OAuth or GitHub App client secret), Azure (an app registration's client secret) and many consoles show a new secret once, often as text with a Copy button, and some offer a key only as a download (a GitHub App's private key `.pem`). Today the agent driving the pane reads it in its next `snapshot`.

**Mask it before the agent can read it.** `snapshot`, `find`, read-backs and screenshots of every pane mask a value with the shape of a one-time secret. The rule, which every pane uses at all times:
- **Shape:** 20 to 512 characters with no whitespace, and at least two of lowercase, uppercase, digits and symbols.
- **Not one of these:** a GUID (an Azure client or tenant id, which isn't secret), pure hex (commit SHAs, which agents need), or a URL.
- **Where it is:** a read-only `input` or `textarea`; a `code`, `pre`, `kbd` or `samp` element; or an element whose own row or nearest container has a button named like "Copy".
- **Also masked:** anything the existing credential patterns in `crates/common/src/redact.rs` match (`ghp_`, `github_pat_`, `sk-` and the rest), wherever it is.

A masked value shows as `[secret: shown once; use Browser capture to save it]`. `eval` and read-only scripts are refused on a tab while such a value is on its page, as for tainted fields (§10.1).

**`Browser` `capture` `{pane, ref, as: {kind, label, fields?, expires?}, entry?}`** reads the element's value (an input's value, otherwise its text content, trimmed) in the host and stores it:
- as a new entry of `kind` (`api-key` by default);
- or, with `entry`, as a new version of that entry (§4.4).

It taints the element and returns the entry's id. The value goes from the renderer to srv through the zeroizing path of §5.1, never through the generic CDP event parse.

**Downloads:** `Browser` `capture` `{pane, download: true, as}` routes the pane's next download, within 2 minutes, into the entry (a `note` by default, its text part holding the file) instead of the person's Downloads folder. The file is never written to disk. `secret_files` (§4.3) gives it back to a command as a file.

**Approval:** none. Saving a value takes nothing out of the Vault, and the page already showed it.

**Where it came from:** the entry keeps the page's origin, taken from the host, as its `service` or `site` field, and the pane shows "captured from <origin> by <agent>". A page can plant only its own value, and the person sees where it came from.

**Recorded:** "captured from <origin>", with the agent and the entry, as for every write.

**Who reviews:** Korp, as for the other browser phases. Tests:
- the shape rule against real GitHub, Azure and AWS console pages, with GUIDs, SHAs and URLs left visible;
- the value never in a snapshot, find, screenshot or log;
- a download never touches the disk.

### 15.2 X1: expiry dates and reminders

**Recommended design:**
- **Kinds with an expiry date:**
  - `api-key` and the ID kinds already have a plain `expires` field. It becomes a validated date (`YYYY-MM-DD`).
  - `login` and `note` gain the same field.
  - A card's expiry stays a sealed part and gets no reminder, since srv would have to open the card to read it on a schedule. The person sees an expired card when it is used.
- **Who sets it:**
  - The person, in the pane.
  - An agent, through `expires` on `generate`, `capture` (§4.2, §15.1) and `rotate` (§4.4), when it knows how long the value lasts (a GitHub token's or an Entra secret's lifetime).
  - Setting or changing it is recorded.
- **Who sees it:** `Vault` `list` gains `expires` for each entry, since a date is no secret. The pane shows "expires in N days" on the entry and offers an "Expiring soon" filter.
- **Reminders:**
  - srv checks at start and every 6 hours.
  - At 14 days, 3 days and on the day, the person gets an AgentMux notification ("Stripe live key expires in 3 days"), and the line is recorded.
  - Each reminder is sent once, so a restart doesn't repeat it.
- **Rotation:**
  - At 14 days, if the entry was made or last rotated by an agent, that agent gets a Kanban task: "Rotate <label>: it expires on <date>". The task names the entry and, after V3, the `rotate` commands last used for it.
  - With no such agent, the task is filed unassigned.
- **Expired entries:** they stay usable, so a reminder never breaks a running job. The pane shows them in the warning colour, and each use of one is recorded with "(expired)".

### 15.3 Recipes for the usual targets (no new action)

Each needs the credentials that permit the change, as running the command by hand would.

- **A GitHub repository or Environment secret.** `gh secret set NAME --repo OWNER/REPO` reads the value from stdin when `--body` is left out, so:
  - run it with `stdin_secret: "<entry>"`;
  - add `--env ENV` for an Environment.
- **A cloud secret store's value** (AWS's, for example): its CLI can read the new value from a file given as a `file://` path, so the entry goes in with `secret_files: {"SECRET_FILE": "<entry>"}` and the command names `file://$SECRET_FILE`.
- **An Entra app's client secret (rotation):**
  - Entra makes the value itself, so the new value is captured, not generated: `az ad app credential reset --id APP_ID --append --display-name NAME --years 1 --query password -o tsv`, with `capture: [{as: {...}, from: {"stdout": "all"}}]` (V2), or as `rotate`'s `set` with that capture (V3).
  - Then the old credential is removed with `az ad app credential delete --id APP_ID --key-id OLD_KEY_ID`. The key id is not secret.
- **A GitHub App's client secret or private key.** GitHub offers no API to make them, so the person or an agent makes one on the App's settings page and captures it from the pane (§15.1): the client secret from the text shown once, the private key from its download.
- **A Docker registry:**
  - run `docker login REGISTRY -u USER --password-stdin`;
  - with `stdin_secret: "<entry>"`.

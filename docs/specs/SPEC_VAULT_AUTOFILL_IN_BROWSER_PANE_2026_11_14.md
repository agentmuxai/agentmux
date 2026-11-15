# SPEC: Vault autofill in the Browser pane: Chrome's password manager, recreated for our Vault

**Status:** draft for the operator. Task #47633; the build is #47634 and starts only after the operator approves this.
**Author:** Agent2-6 (Vault). **Engine side:** Korp (asked 2026-11-14; §P2.2 says what is confirmed and what is still open). **Browser pane:** AgentX (told it exists).
**Verified on:** `origin/main` `090eefdc8` (2026-11-14).
**The operator's words, 2026-11-14, verbatim:** "lets write a spec that recreates the chrome password manager in chrome browsers, but for our vault inside of the agentmux browser pane, esscially, auto complete on the password fields, lets research how modern chrome does it and try to replicate as close as possible".
**Extends:** `SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md` (called "the sign-ins spec" below), which already saves sign-ins, fills them from a key button's menu, makes strong passwords and lets agents ask. This spec adds the part that is missing: **the dropdown in the field.**

## 1. Summary

Click a username or password field in a Browser pane and a list opens under it with the Vault's saved sign-ins for that page, as in Chrome. Pick one and the fields fill. On a sign-up form the list also offers "Use a strong password". The Vault is the only store; Chromium's own password manager stays off.

What is the same as Chrome: when it opens (a click or an arrow key in the field, not Tab), what it lists and in what order, the typed-text filter, "Manage passwords" at the foot, the keyboard, saving after a sign-in, "Update password?", "Never for this site", a strong password on new-password fields.

What is different on purpose, and why (details in Part 1's tables and Part 2):
- **Nothing is filled until the person picks.** Chrome fills a single saved sign-in as the page loads. Here the value reaches the page only by the person's choice.
- **The list is drawn by the host, outside the page,** and an agent that drives the pane never sees it, never presses a row and never gets a value.
- **Exact origin only** to start with (scheme, host and port), with no "sibling site" matches.
- **No window bubbles.** Save and update are a band in the pane's chrome, as built.
- **Sign-ins belong to a browser profile,** and a container agent's profile never offers the person's own.

## 2. How to read Part 1

Part 1 is the research: how current desktop Chrome behaves, read on 2026-11-14 from Chromium's own source, Chrome Help, web.dev, developer.chrome.com and the WHATWG and W3C specs, not from memory. Every Chrome line is marked **confirmed** with its source or **not confirmed**. The Chromium source is `main` as fetched on 2026-11-14 (cluster A read at commit `bc619944`), which carries no Chrome version number, so read it as "current development head, roughly the Chrome 14x line". Chrome Help, web.dev and developer.chrome.com pages show no date. Sources older than two years are marked **old**. Quotes taken from a summarising fetch rather than the raw file are marked **(fetch)** in the research notes and are not repeated here.

Source key (every Chromium file is under `https://chromium.googlesource.com/chromium/src/+/main/`):
- **PM** = `components/password_manager/core/browser/`
- **AR** = `components/autofill/content/renderer/`
- **CB** = `chrome/browser/`
- **GR** = `chrome/app/generated_resources.grd`
- **Help** = `https://support.google.com/chrome/answer/95606` (manage passwords), `/10311524` (breach warning), `/13168025` (passkeys), `/13068232` (import and export), `/10468685` (Safety Check)
- **WHATWG** = `https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#autofill`

The last column says what AgentMux does: **same**, **different on purpose** (with the reason), or **later** (with the phase of §P2.8). "Today" means already built (the sign-ins spec, "as built").

The raw notes, with the verbatim quotes and the full URLs, are in the author's workspace (`research-47633/A.md`, `B.md`, `C.md`); they are working files and are not committed.

# Part 1. How Chrome's password manager behaves

## 1.1 Offering a saved sign-in

| # | Chrome | Status and source | AgentMux |
|---|---|---|---|
| 1 | With one saved sign-in Chrome fills the form with no pick; with several, the person selects the username field and chooses one | confirmed: Help 95606 ("Chrome will fill in the sign-in form automatically"; "Select the username field and choose the sign-in info you want to use") | **Different on purpose** for the single case: nothing fills until the person picks (a value in the DOM is readable by the page's script). The several-accounts case is the same. Opt-in fill on first gesture is **later** (P4) |
| 2 | Per form the browser process decides "fill on page load" or "fill on account select". Any wait reason gives account select | confirmed: PM `password_form_filling.cc` | Always "on account select" in P1 |
| 3 | Reasons that stop page-load fill: Incognito; biometric re-authentication on; affiliated-site, public-suffix or grouped match; cross-origin iframe; no current-password field; a username field with a non-empty password already; insecure main frame; fill-on-account-select feature; form accepts WebAuthn; single-username form; password change | confirmed: PM `password_form_filling.cc` | Not applicable in P1 (never page-load). Each of these is a choose-from-the-list case anyway |
| 4 | The list opens when a focused text field is clicked or tapped; Arrow Down or Arrow Up in a text input opens it and the key is not swallowed; Tab focus alone does not open it | confirmed: AR `autofill_agent.cc` | **Same** |
| 5 | The list can open on page load for an autofocused sign-in field; on by default on desktop since M140 | confirmed: PM `core/common/password_manager_features.cc`; AR `password_autofill_agent.cc` (shows only when the form's fill data arrived exactly once) | **Later** (P4): a list over a page the person has not touched is held back until the click-opened one has settled |
| 6 | No list for a field that is not editable, whose text is over 1000 characters, or when no username or password field is fillable | confirmed: AR `password_autofill_agent.cc` | **Same** |
| 7 | On a password field, no second list if the person already accepted a password suggestion on another password field | confirmed: AR `password_autofill_agent.cc` | **Same** |
| 8 | On a username field the list is filtered by the typed text (case-insensitive prefix of the saved username); nothing shows if all are filtered | confirmed: AR `password_autofill_agent.cc`; PM `password_suggestion_generator.cc` | **Same** |
| 9 | Order: the preferred match first, then the other accounts sorted by username text, then a "trouble signing in" row if one applies. The preferred match is ranked by match type, then last-used date | confirmed: PM `password_suggestion_generator.cc`, `password_manager_util.cc` | **Same** order (most recently used exact match first, then by username); no recovery row |
| 10 | Each row: the username as main text and the password as bullets of the same length as the password; an empty username shows "No username" | confirmed: PM `password_suggestion_generator.cc`; `components/password_manager_strings.grdp` | **Different on purpose:** the mask is a fixed length (the length is not shown); usernames masked as today (`al***`) until the operator decides §P2.10 question 1; "No username" is the same |
| 11 | Whole list order: passkeys, federated credentials, passwords, "Use strong password", a re-authentication promo, the footer | confirmed: PM `password_suggestion_generator.cc` | Passwords, then "Use a strong password" on a new-password field, then the footer. Passkeys, federated: not in this spec |
| 12 | Footer: a separator and "Manage passwords" (or "Manage passwords and passkeys"), static (the typed filter keeps it); added only if at least one fillable row exists; a list whose only row is "Manage passwords" is not shown | confirmed: PM `password_suggestion_generator.cc`, `password_autofill_manager.cc` | **Same.** "Manage passwords" opens the Vault, filtered to the page's site |
| 13 | Keyboard in the list: Down and Up move; Page Up and Page Down clear the selection and move; Left and Right move between cells or into a sub-list; Shift+Delete removes the selected suggestion; Escape closes a sub-list, then the list; Tab or Shift+Tab accepts the selected row and still moves focus; Enter accepts | confirmed: CB `ui/views/autofill/popup/popup_view_views.cc`, `popup_row_view.cc` | **Same** for Up, Down, Page Up, Page Down, Enter, Escape, Tab. **Not built:** Shift+Delete (deleting a Vault entry stays in the Vault), sub-lists |
| 14 | The list is a native widget drawn by the browser process, not in the page's DOM; the browser registers a key handler on the render frame host, so the page's field keeps focus while the browser takes the keys; an unhandled key goes on to the page | confirmed: CB `ui/views/autofill/popup/popup_view_views.cc`, `ui/autofill/autofill_popup_controller_impl.cc`; PM `password_suggestion_generator.cc` (rows are `Suggestion` objects, not DOM) | **Same shape:** drawn by the host outside the page (§P2.2) |
| 15 | What page script can observe of the list (events on the field after a pick, no DOM node, no blur) | **not confirmed**: no document read states it | Designed so the page learns only what a person's own typing would tell it (§P2.4) |
| 16 | The address-bar key icon is the anchor of the save bubble and shows "Save your password" / "Manage your passwords" / "Never for this site" | confirmed: CB `ui/views/passwords/password_bubble_view_base.cc`; GR | **Different on purpose:** the pane has a key button in its own chrome (today) and no bubble |
| 17 | An in-field key icon, the list's layout (widths, fonts, row heights), the history of page-load fill rules | **not confirmed** | Our own look, in the kit's styles |
| 18 | Re-authentication before a fill (Windows Hello, Touch ID or the OS password) is an opt-in setting, off by default; with it on, no page-load fill and no preview; picking a row asks the OS first | confirmed: Help 95606; PM `password_form_filling.cc`, `password_autofill_manager.cc`; CB `chrome_password_manager_client.cc` | **Later** (P3), off by default, as Chrome |
| 19 | "Sign in automatically" skips the confirmation step of the Credential Management API | confirmed: Help 95606 | Not in this spec (§P2.9) |

## 1.2 Recognising fields

| # | Chrome | Status and source | AgentMux |
|---|---|---|---|
| 20 | The spec defines `username`, `current-password`, `new-password`, `one-time-code`; `webauthn` is a last token that asks for passkeys through conditional mediation | confirmed: WHATWG; web.dev `passkey-form-autofill` | The listener reads the first four (today). `webauthn`: passkeys are a non-goal |
| 21 | Chrome takes the last token of the attribute, recognises those four (case-insensitively), and any `cc-` token as a card field | confirmed: PM `form_parsing/form_data_parser.cc` | **Same** (the listener's rule is in `signin_notice.js`; a parity test pins it to `signup_probe.rs`, #7299) |
| 22 | Sources of truth, most trusted first: a classification model, server predictions, autocomplete attributes, local heuristics, then a clean-up | confirmed: PM `form_parsing/form_data_parser.cc` | **Simpler on purpose:** attributes, then heuristics. No model, no server (no third party sees a page) |
| 23 | Attribute use is all-or-nothing under four assumptions (at most one username, one current-password, two new-password; `*-password` only on password inputs, `username` only on non-password inputs); a violation drops the attributes | confirmed: PM `form_parsing/form_data_parser.cc` | **Later:** the build checks the listener against these four assumptions and says what differs |
| 24 | First new-password field is the new password, the second its confirmation; two identical non-empty values mean new plus confirmation; two different mean old then new; empty ones err toward old-and-new so change forms still fill | confirmed: PM `form_parsing/form_data_parser.cc` | **Same** (a form with `current-password` and a new one is a password change; §7.3 of the sign-ins spec) |
| 25 | `one-time-code` and fields named like a CVC, SSN or one-time password are not passwords | confirmed: PM `form_parsing/form_data_parser.cc` | **Same** (`password_like()` already refuses them) |
| 26 | Heuristic username: the closest field before the first password, preferring the closest focusable one; fields the person typed in or that are focusable outrank the rest, so hidden fields lose; read-only password fields are ignored unless typed or filled earlier | confirmed: PM `form_parsing/form_data_parser.cc` | **Same,** with a stricter visibility rule (row 49) |
| 27 | Username-first flows: a username with no password field is a "single username" form; text typed outside the password form is kept as a possible username for 5 minutes, at most 10 candidates | confirmed: PM `form_parsing/form_data_parser.cc`, `possible_username_data.h`, `core/common/password_manager_constants.h` | **Same:** the picked sign-in is remembered for 5 minutes per tab and origin, so the password step offers it first |
| 28 | Fields without a `<form>` are treated as one synthetic form | confirmed: AR `form_autofill_util.cc` | **Same** (today) |
| 29 | Forms with more than 10,000 fields are not parsed | confirmed (header read through a fetch): PM `form_parsing/form_data_parser.h` | Our own caps (frames per tab, scan rate) |
| 30 | `autocomplete=off` on a password field is ignored since Chrome 34 | confirmed, **old** (2014 announcement); not confirmed for current Chrome. Its effect on a non-password username field in the list: not confirmed | The list ignores `off` on password fields, since the person chose to keep the sign-in |
| 31 | When script-built or formless forms are re-parsed after DOM changes | **not confirmed** | A scan after DOM changes, at most every 500 ms (today) |

## 1.3 Which saved entry matches a page

| # | Chrome | Status and source | AgentMux |
|---|---|---|---|
| 32 | An entry's identity is its sign-on realm: scheme, host and port. Equal realms are an exact match; anything else is tried as public-suffix, then federated | confirmed: PM `password_form.h`, `password_store/psl_matching_helper.cc` | **Same for exact:** a login is offered only to a frame whose origin is exactly its site |
| 33 | Public-suffix match: same scheme, same port, same registrable domain (private registries count as suffixes); for google.com only `myaccount` and `accounts` match each other; only for HTML forms | confirmed: PM `password_store/psl_matching_helper.h`, `.cc` | **Later** (P4): shown in the list with the other site's host as a label, never filled as the page loads (Chrome too), and only after the person picks |
| 34 | Matches are ranked exact, affiliated, public-suffix, grouped; public-suffix and grouped are "weak" | confirmed: PM `password_manager_util.cc` | Exact only until P4 |
| 35 | A weak match is never filled on page load; a non-exact row shows the other site's name as a label; signing in with one saves a copy for the current origin without asking | confirmed: PM `password_form_filling.cc`, `password_suggestion_generator.cc`, `password_manager.cc`, `password_save_manager_impl.cc` | P4 shows the label and asks before saving a copy; Chrome saves it silently |
| 36 | Affiliated sites and apps: groups of web sites and apps that share a login ("facets"), declared with Digital Asset Links (`delegate_permission/common.get_login_creds`) | confirmed: PM `affiliations/core/browser/affiliation_utils.h`; developer.chrome.com `site-affiliation`. Where Chrome fetches the data and how often it refreshes: **not confirmed** | The person's own "also use on" origins per entry (the sign-ins spec §5.3, not built) is the P4 equivalent. No third-party affiliation data |
| 37 | HTTP and HTTPS: public-suffix needs the same scheme; a plain-HTTP main frame is never pre-filled (choices only); saving is blocked when the host that just submitted over HTTPS submits over HTTP; a migrator can move or copy HTTP entries to HTTPS | confirmed: PM `password_manager.cc`, `password_form_filling.cc`, `http_password_store_migrator.h`. Whether an https page is offered an http-saved entry: **not confirmed** (the matching code gives no match) | **Same or stricter:** no list on plain `http` except `localhost`, `127.0.0.1` and `[::1]` (today), and an http-saved entry is not offered on https |
| 38 | A different port is a different site | confirmed: PM `psl_matching_helper.cc` | **Same** |
| 39 | Frames: filling is decided per frame from that frame's own origin; a frame with a cross-origin ancestor is not pre-filled and is offered the list; the renderer logs a block for filling in a cross-origin iframe; Chrome's cross-frame Autofill policy excludes passwords; a WHATWG contributor says Chrome does not fill passwords across frames | confirmed: PM `content/browser/content_password_manager_driver.cc`, `password_form_filling.cc`; AR `password_autofill_agent.cc`; `docs/security/autofill-across-iframes.md` (tag 138, older); whatwg/html PR 8801 (one contributor's statement). Whether a pick fills inside a cross-origin iframe and how an iframe form's realm is built: **not confirmed** | **Our rule is simpler:** a frame is offered exactly the logins saved for that frame's own origin (the host's frame map, never the page's word), so a login iframe of another site gets its own site's sign-ins only |
| 40 | Opaque origins (sandboxed frames, `data:`), pages with certificate errors, guest profiles: no fill and no save | confirmed: PM `password_manager_client.h`; CB `chrome_password_manager_client.cc` | **Same** (opaque origins refused; a certificate-error page is never shown, today) |
| 41 | Federated entries live under `federation://<site>/<provider>`; a flag can drop the save prompt on a "Sign in with" login | confirmed: PM `psl_matching_helper.cc`, `password_form.h`, `password_manager.cc`. The flag's default, the list's rows for them and FedCM interplay: **not confirmed** | Not in this spec |

## 1.4 Saving

| # | Chrome | Status and source | AgentMux |
|---|---|---|---|
| 42 | After a sign-in a bubble anchored to the key icon asks "Save password?" with Save, "Never" (or a split "Not now" with "Never for this site"); username and password are editable with a show toggle; a destination menu ("Save in your Google Account" or "Save only on this device") | confirmed: GR; CB `ui/views/passwords/password_save_update_view.cc` | **Different on purpose:** a band in the pane's chrome (no bubble over the window), Save, "Never for this site", "Not now", as built. No edit fields in the band (edit in the Vault). The destination is the profile's Vault |
| 43 | "Update password?" always asks; with several accounts it lists them to choose | confirmed: PM `password_manager.cc`; CB `password_save_update_view.cc` | **Same** (as built, with the old value kept as a previous version) |
| 44 | A new login prompts; a generated password is saved without a prompt, with a confirmation after; a submission counts only with a non-empty password that is not a one-time code; the value saved on a change-password form is the new password | confirmed: PM `password_manager.cc`, `password_form_manager.cc`, `password_save_manager_impl.cc` | **Same** (as built: the made password is in the Vault before the form is sent) |
| 45 | "Never" writes a blocklist record for the exact realm and scheme; matches of other kinds ignore it; the person manages the list as "Declined sites and apps" | confirmed: PM `form_saver_impl.cc`, `password_manager_util.cc`, `form_fetcher_impl.cc`; Help 95606 | **Same,** kept per profile and exact site (as built, `db_browser_signin_never`). The list in Settings: P2 |
| 46 | A sign-in succeeded when: after a navigation the form is not seen again; no HTTP error page; the submitted form was removed from the DOM; or one of these signals fired: HTML form submission, same-document navigation, XHR succeeded, frame detach, a probable submission (a main-frame navigation not caused by a link), change-password form cleared, DOM mutation after autofill. A formless form needs input in every removed password field | confirmed: PM `password_manager.cc`; `components/autofill/core/common/mojom/autofill_types.mojom`; `components/autofill/README.md`; web.dev `sign-in-form-best-practices` | **Mostly the same:** a submit by the person's own click or key press within 2 seconds, settled by a navigation, a new document in the frame, or the password field leaving the page (as built). **Not built:** the XHR-succeeded and DOM-mutation signals, and the HTTP-error-page check (later, P2) |
| 47 | Not offered in Incognito, off the record, with "Offer to save" off, or in automation-driven browsers | confirmed: CB `chrome_password_manager_client.cc`; PM `password_form_manager.cc` | Incognito never offers Save (as built). On a pane an agent drives, the offer goes only to the person (as built) |
| 48 | The manual fallback: for a field that was not recognised as a login form, the key icon appears with no pop-up and a 90-second timer; a prompt pops up only when it can be shown | confirmed: CB `ui/passwords/manage_passwords_ui_controller.cc` | **Later** (P4) |
| 49 | What an entry stores: realm, URL, form action, usernames and element names, dates created, last used and modified, notes, a type (form submission, generated, API, manually added, imported), federation origin, blocklist flag | confirmed: PM `password_form.h` | A `login` entry: site, username, password, one-time-code secret, notes, tags, profile, with versions (the Vault's) |
| 50 | The number of dismissals after which the save prompt stops; the "Add username?" bubble when no username was found | threshold **not confirmed**; the bubble confirmed: GR | **Later:** the Vault's Save band offers "Add a username" when none was found |

## 1.5 Generating

| # | Chrome | Status and source | AgentMux |
|---|---|---|---|
| 51 | The list's row reads "Use strong password" (text: "You won't need to remember this password. It will be saved to…"); the context menu has "Suggest password…"; the password is saved to the Google Account | confirmed: GR; Help 7570435 | **Same row,** reading "Use a strong password (saved to the Vault)" (today, in the key menu). Saved to the Vault, never a Google account |
| 52 | Offered when the parser is confident a new-password field exists and the site is not blocklisted; not on google.com; only when saving is enabled | confirmed: PM `password_form_manager.cc`, `password_generation_frame_helper.cc` | **Same** (a field with `new-password`, two password fields, or a "Create account" form, as built) |
| 53 | Accepting fills every password field of the form (new and confirmation); edits are mirrored to the confirmation fields | confirmed: AR `password_generation_agent.cc` | The new and confirm fields fill (as built). Mirroring a later edit: **later** |
| 54 | Defaults: 15 characters; letters and digits without look-alikes, symbols off; at least one lower, upper and digit; length is the least of the default, the field's `maxlength` and a server hint, capped at 200; a hint with its own character sets, or a maximum under the minimum, is dropped | confirmed: PM `generation/password_generator.cc`, `password_generation_frame_helper.cc` | **Different on purpose:** 20 characters under the field's `passwordrules`, `minlength` and `maxlength` (as built). That Chrome honours `minlength`, `pattern` or `passwordrules`: **not confirmed** (the code read uses `maxlength` and per-domain specs from gstatic only; a third party's comment says it ignores `pattern` and `minLength`) |
| 55 | The password is saved before the form is sent ("presave"); edits replace it in place; under 4 characters it stops counting as generated and is rolled back; if page script clears the field, nothing is presaved | confirmed: AR `password_generation_agent.cc`; PM `password_generation_manager.cc`, `password_save_manager_impl.cc` | **Same** (made, stored unconfirmed, then typed) |
| 56 | If the site rejects the password after submit | **not confirmed** | Choose the row again: a new version replaces the unconfirmed one (as built) |

## 1.6 Protection

| # | Chrome | Status and source | AgentMux |
|---|---|---|---|
| 57 | Re-authentication: the setting reads "Use Windows Hello when filling passwords" (Windows) or "Use your screen lock when filling passwords" (Mac); not on Linux, where no re-authentication happens at all. Show, copy and edit in the manager are valid for 5 minutes; export and import ask fresh every time. The OS prompt covers HTTP-auth dialogs and Credential Management results too | confirmed: Help 95606; `chrome/app/password_manager_ui_strings.grdp`; CB `chrome_password_manager_client.cc`, `extensions/api/passwords_private/passwords_private_delegate_impl.cc`; PM `http_auth_manager_impl.cc`, `credential_manager_impl.cc` | The Vault's reveal and copy already need the OS to confirm the person (host-only). The fill setting: **later** (P3), Windows and macOS only, off by default |
| 58 | On page load only a suggested value is set and the real value goes into the DOM after the first user gesture; page-load fill skips fields the person typed in; a pick from the list is a gesture | confirmed: AR `password_autofill_agent.cc` | **Different on purpose in effect:** there is no page-load fill, so the value is set only by the person's pick |
| 59 | `:-webkit-autofill` matches an autofilled input and stops when the person edits it | confirmed: MDN `:autofill`; WHATWG. That the class hides the value from script: **not confirmed** (the hiding is the gesture gate) | A filled field also carries `data-agentmux-vault`, so an agent's snapshot and screenshot never show it (today). Chrome has no equivalent |
| 60 | Hidden and invisible fields: the fill code's only gate is "enabled and not read-only", with no visibility test; whether current Chrome refuses an invisible password field is unresolved | confirmed for the code read: AR `form_autofill_util.cc`, `password_autofill_agent.cc`; the bug-tracker pages could not be read | **Different on purpose:** a field must be shown, with no hidden ancestor, not clipped and on top (`elementFromPoint`), or nothing is typed (today; a hidden field is how a page steals a fill) |
| 61 | Cross-origin frames: see row 39 | | |
| 62 | Leaked, weak and reused checks: Password Checkup (compromised, reused, weak) and a warning after signing in with a leaked password; username hash prefix plus a re-encrypted lookup (the match is decided on the device); scrypt, not Argon2; skipped in Incognito and for an empty username; the Safety Check includes a passwords row | confirmed: Help 10311524, 10468685; PM `leak_detection/leak_detection_request_utils.cc`, `leak_detection_check_impl.cc`, `leak_detection_delegate.cc`, `leak_detection_dialog_utils.cc`; CB `resources/password_manager/checkup_section.html`. The prefix length today: **not confirmed** (3 bytes in a 2019 post, **old**) | **Later,** and its own spec: it needs a service outside AgentMux, which this one does not add. Weak and reused checks can run locally with no service |

## 1.7 The rest

| # | Chrome | Status and source | AgentMux |
|---|---|---|---|
| 63 | HTTP authentication dialogs: saved after the dialog and a next navigation that is not an HTTP error; filled after re-authentication when it is on | confirmed: PM `http_auth_manager_impl.cc` | **Today:** the "Use saved sign-in?" window and `login` entries with a realm (the sign-ins spec §5.4) |
| 64 | Passkeys: kept in Google Password Manager, Windows Hello, macOS or a security key; shown with passwords in the list through the `webauthn` token; "Use a passkey from another device" and a QR code | confirmed: Help 13168025; web.dev; developer.chrome.com `webauthn-conditional-ui`; PM `password_autofill_manager.cc` | Not in this spec (a non-goal of the sign-ins spec) |
| 65 | Export columns `name, url, username, password, note`; import accepts header aliases (`url`, `website`, `origin`, `hostname`, `login_uri`; `username`, `user`, `login`, `account`; `password`; `note`, `notes`, `comment`) and needs exactly one of each of the three; the help says up to 3,000 per file; both ask the OS fresh each time | confirmed: PM `export/password_csv_writer.cc`, `import/csv_password_sequence.cc`; Help 13068232. Quoting and escaping of the writer, size limits in code: **not confirmed** | **Today:** the Vault imports Chrome's CSV and 20+ other sources, and exports encrypted or plain (#7265, #7298) |
| 66 | Notes on an entry have the protection of the password; the key icon is pointed out after a save | confirmed: Help 95606; GR | **Same:** sealed with the entry |
| 67 | The Settings page: save passwords, automatic passkey upgrades, sign in automatically, re-authentication (Windows, Mac, ChromeOS), import, export, "Add shortcut", change the PIN, manage passkeys (Windows, Mac), delete all data, and the "Declined sites and apps" list | confirmed: CB `resources/password_manager/settings_section.html`; `password_manager_ui_strings.grdp` | A "Saved sign-ins" group in Settings > Browser (P2): the offer, strong passwords, declined sites (P2); device sign-in (P3) |
| 68 | Sync between devices, signed in; signed out stays local | confirmed: Help 95606 | The Vault is per machine; no sync here |
| 69 | The Credential Management API: auto sign-in only for an exact match, one match, zero-click allowed and the origin not marked "requires mediation"; otherwise the account chooser; incognito returns nothing | confirmed: `credential_manager_impl.cc`, `credential_manager_pending_request_task.cc`; MDN; W3C credential-management-1. Whether current desktop Chrome still returns password credentials: **not confirmed** | Not in this spec |
| 70 | Chrome's handling of "Use a different username", the in-field key icon, the dismissal limit, enterprise policies, family sharing | **not confirmed**, or no such behaviour found | None of them |

### What the research could not settle (all marked above)

The in-field and address-bar key-icon visuals; what page script can observe of the list; whether Chrome refuses invisible password fields; whether Chrome honours `minlength` and `passwordrules` when it generates; what Chrome shows when a site rejects a generated password; the source and refresh of affiliation data; whether a pick fills inside a cross-origin iframe; the save prompt's dismissal threshold; the current username-hash prefix length. None of them changes the design below: where a row is "not confirmed" the AgentMux column is chosen to be safe either way.

# Part 2. What AgentMux builds

## P2.1 What exists, and the gap

Built today (the sign-ins spec, "as built"): an isolated-world listener (`signin_notice.js`) that finds login and sign-up forms; `signup_probe.rs`, which its parity test pins to the listener's rules; a key button in the pane's own chrome with a menu only the person can use; `browser.signin_fill`, which types a saved sign-in into the page from the host; the save band (`browser:signin_offer`) with Save, "Never for this site" and "Not now"; `generate_password`; browser profiles that own their sign-ins; the Chrome importer (#7265) and the other importers (#7298).

The gap is the one the operator named: **a person who clicks into a password field gets nothing under it.** They have to find the key button, open its menu and choose. Chrome's whole experience is the list in the field (rows 4 to 14 of Part 1). Everything behind the list (storage, matching, saving, generating, filling) exists. This spec adds the list, and the rules that keep it from becoming a way for a page or an agent to read a password.

## P2.2 Engine route: how the list gets drawn

The list has to be drawn **outside the page** (row 14), over the pane, and take the arrow keys while the page's field keeps focus. Four routes were weighed:

| Route | What it is | Verdict |
|---|---|---|
| **R1** | Turn on Chromium's own password manager and let it draw its list | **Rejected.** It stores in Chrome's own store and Google account, not the Vault; its rows, matching and saving are not ours to change; and our rules (nothing fills until a pick, no agent can reach it) could not be enforced |
| **R2** | **The host draws a native popup window over the pane, from the Vault's rows; our own detection (the listener) decides when and for which field** | **Recommended.** It is the same shape as Chrome's (row 14), the page cannot see or reach it (no DOM node), the Vault stays the only store, and the key handler lives in the host so the page's field keeps focus |
| **R3** | Draw the list as page DOM (an overlay in the page, or in the isolated world) | **Rejected.** Page script can read, click and style anything in its own document, and an overlay changes what a screenshot or snapshot shows. It would also leak the rows to any agent that reads the page |
| **R4** | The list in the pane's own chrome strip (under the key button) rather than under the field | **Fallback only.** Safe, but far from the field and not what the operator asked for. It is what ships if R2 cannot be done on a platform |

**What is confirmed and what is not.** Confirmed: the pane's own chrome is host-drawn and sits outside the page (it is how the key button and the save band work today). **Asked of Korp (the engine seat) on 2026-11-14, and answered from the code below; not yet confirmed by a run:** whether the CEF build can show a native popup that (a) takes Up, Down, Page Up, Page Down, Enter, Tab and Escape without the page's field losing focus, (b) is positioned from the field's rectangle and follows scroll, zoom and resize, (c) is not captured by a screenshot an agent takes of the pane, and (d) works on Windows, macOS and Linux. R2 stands as the recommendation until Korp answers; if any of a to d is "no" on a platform, that platform ships R4 until it is fixed. The answer goes here when it arrives.

**The answer must name the pane path, per platform** (AgentX's review, 2026-11-14). A browser pane is not one thing: on Windows the default is the child-window kind (a window of AgentMux's own, with its own subclass), the Views kind is opt-in (#7069, held), and macOS and Linux each differ again. Korp's a to d is therefore a table, one row per path, and the result for each row is R2 or R4:

| Path | a: keys without losing focus | b: follows the field | c: absent from an agent's screenshot | d: available | Route |
|---|---|---|---|---|---|
| Windows, child-window pane (default) | untested: a no-activate popup plus consuming the keys in the client's key handler (`OnPreKeyEvent`) before the page sees them | by composition: the field's rectangle in the frame's own world plus each frame owner's offset | not stated | partly: the pane is a native child window, so the host UI beneath it cannot draw over it; the list must be its own native window above the pane | R4 until a run confirms |
| Windows, Views pane (opt-in) | as above, untested | as above | not stated | not stated for this path (Korp named the Views overlay for Linux and macOS) | R4 until a run confirms |
| macOS | untested, as above | as above | not stated | likely: the pane is a Views overlay, and a second overlay view above it works | R4 until a run confirms |
| Linux | untested, as above | as above | not stated | likely: the pane is a Views overlay, and a second overlay view above it works | R4 until a run confirms |

The table is filled in by Korp's answer, not by this spec. A path whose row is not "yes" on all four ships R4, so P1 never waits on the hardest platform.

**Korp's answer (2026-11-14), from the code, nothing run.** Paths are under `crates/cef/src`.

1. **Chromium's own manager (R1) stays rejected.** Browser panes are Alloy-style (`browser_pane/creation.rs:210` on Windows, `creation_views.rs:201` on Linux and macOS), so there is no password-manager UI to show. The host also forces the prefs off in every profile (`browser_pane/signin_notice.rs:282-306`: `credentials_enable_service`, `profile.password_manager_enabled`, `autofill.profile_enabled`, `autofill.credit_card_enabled`) and disables AutofillServerCommunication (`app/mod.rs`, near line 799). Chrome style would bring Chromium's own UI and toolbar with it; Korp would not switch for this.
2. **What can be drawn over a field.** No pane uses off-screen rendering, so there is no windowless popup to use. On Windows the pane is a native child window (`WindowInfo::set_as_child`), so anything over the field is its own native window above the pane. On Linux and macOS the pane is a Views overlay (`add_overlay_view`), so a second overlay view above it works. **An injected frame in the page is not a trust surface** (the page's own compositor draws it; it can restyle, cover or click it), which keeps R3 rejected. The panel that exists today is the pane header's key button and "Sign in as" menu, outside the page. The listener runs in an isolated world in every document, frames included, reports through a binding only that world has, and the host names the frame origin from its own world map; cross-origin frames already work. The field's rectangle in top-frame coordinates is composed by the host (frame owner quads per level over CDP), with no page script involved.
3. **The page seeing a blur.** A popup that takes OS focus makes the page see a blur. A no-activate popup that consumes the keys in the client's key handler before the page gets them would hide it; `input_owner/` already routes focus between panes and the host. **Untested.**
4. **Trusted focus.** Focus alone is not trusted: a page's own `el.focus()` raises a focus event with `isTrusted` true. The rule to copy is `signin_notice.js`'s: count a field only when the person's own `pointerdown` or `keydown` (trusted) came within about 2 seconds before it (§P2.4). The rectangle is `getBoundingClientRect` in the frame's own world plus each frame owner's offset; an out-of-process frame has its own session, which `signin_notice` already tracks (session, context, frame, origin).
5. **Filling without the page telling it from typing.** Two paths exist. `key_typing.rs` sends per-key `Input.dispatchKeyEvent` (trusted keydown, keyup and input, apart from timing). `insertText` (`browser_api/act.rs`) gives a trusted `insertText` input event with no key events, which a page checking for keydown can tell. `SET_VALUE` and `signup_fill`'s setter path dispatch synthetic input and change events (`isTrusted` false), which a page can see. **Known holes either way:** the fields get a page-readable attribute (`data-agentmux-vault` on Vault-filled fields, `data-agentmux-filled` on any agent-written field), and a fill is instant. If the page must not know, use the per-key path and accept its slowness; the attributes would have to move to a host-side set. This bears on §P2.4's rules and on decision 5 of P2.10.
6. **Three things to test first,** all needing a run and so the operator's OK (labs): **hibernation** (the renderer is discarded, so a popup must close with it; `hibernation.rs`), **Windows native occlusion tracking** (a native window over the child window can mark the pane occluded and throttle its painting; the history is in `docs/reports`), and **transparent windows** (alpha-capable windows change how overlays composite).

**What this changes.** Nothing is confirmed by a run, so every path stays **R4 until a run confirms**. The two paths Korp's reading favours for R2 are macOS and Linux (a second overlay view); Windows needs its own native window and the three tests above. P1 (the list in the chrome strip) does not wait on any of it.

## P2.3 The flow

1. **Click or an arrow key in a login field** (not Tab: row 4). The listener, in the isolated world, reports to the host only *which* field (a frame and a field handle), the form's role (username, current password, new password) and the field's rectangle. It sends no values and no page text.
2. **The host asks the Vault's matching** for that frame's own origin (the host's frame map, never the page's word: row 39), in the pane's profile, and gets a list of rows: username, entry id, last-used time. The password is **not** fetched yet.
3. **The host draws the list** (R2) under the field: rows in the order of row 9, the typed-text filter of row 8, "Use a strong password (saved to the Vault)" on a new-password field (row 51), a separator and "Manage passwords" (row 12). If there is no row to fill and no strong-password row, nothing is shown (row 12).
4. **The person picks** (click, Enter or Tab: row 13). Only now does the host fetch the value, and it types it into the page through `browser.signin_fill`'s existing path (visible, enabled, on top, not read-only: row 60). A fill sets `data-agentmux-vault` (row 59) so the value is not in any snapshot an agent takes.
5. **A username step** remembers the pick for 5 minutes per tab and origin, so the next step's password field offers that sign-in first (row 27).
6. **Closing:** Escape, a click elsewhere, a scroll that moves the field out of view, a navigation, or the pane losing focus. A closed list leaves the field's focus and text as they were.
7. **The sign-in is submitted** by the person; the save band's rules (row 46) take over (P2.5).

## P2.4 Rules Chrome does not have

These exist because an agent shares the pane. Each is a requirement of the build, with a test (P2.11).

1. **The list is for the person's own input.** It opens only on a click or key the person made, not on one an agent sent through the Browser tool.
   - **What "an agent's action" is in the code.** The Browser tool's click is `Input.dispatchMouseEvent` (mouseMoved, mousePressed, mouseReleased) sent over the DevTools protocol (`crates/cef/src/browser_api/act.rs`, `click_at`; `routes.rs`, `click_element`). It is synthetic input handed to the page's renderer. The page sees an ordinary click on its field, and the field gets the same focus and click events as for a person's, so **the page's own side cannot tell them apart, and neither can the listener in the isolated world.**
   - **What the host can see.** On Windows, the host already watches the UI thread's message queue for real button-downs inside a pane (`crates/cef/src/input_owner/native_click.rs`, a `WH_GETMESSAGE` hook, built for #47567). A DevTools click is not a message in that queue, so a button-down seen there is a person's (or another real input device's). That is read from the file's own header and not tested here, it covers Views panes as that file says, and **it is not confirmed for the child-window pane, macOS or Linux.** Keys have no such reading in this spec: **not confirmed** for any path.
   - **So the design is this, not a fallback:** the list opens only while the host has no agent action in flight on that pane, and not within a short moment after one has finished (the exact moment is a build detail). A per-path real-input signal (the Windows queue hook above, and the equivalents Korp finds) *narrows* that rule where it exists, so a person can click while an agent works elsewhere on the page's tab; it does not replace it. A path with no such signal uses the in-flight rule alone.
2. **An agent never sees the list or a value.** No row, no username and no count is in any Browser tool answer, snapshot, element ref or screenshot (R2's item c above). An agent that wants a sign-in uses the Vault's request, which asks the person.
3. **A page cannot tell the list is open.** No blur, no focus change, no DOM node. After a pick, the page sees what it would see if the person had typed: input and change events on the field, nothing else. It cannot tell *which* entry was picked, only the value it was given.
4. **A page cannot make the list open or pick for the person.** Events a page dispatches itself are untrusted and ignored. The list never opens on `focus()` called by script.
5. **A fill happens once per pick.** Page script that clears and refills, or a form that re-renders, does not make the host fill again.
6. **Frames:** a frame is offered the sign-ins saved for its own origin and nothing else (row 39). A cross-origin frame inside a page gets its own site's rows, never the parent's.
7. **No list on a pane an agent holds exclusive** (a tab an agent has opened for itself): such a pane has no person's sign-ins in it anyway (the profile rule below).
8. **Profiles:** the list reads the pane's profile only. A container agent's pane never lists the person's Personal sign-ins (the Container agents rule); an agent's own profile lists only what was saved into it.

## P2.5 Saving and updating

The save band and "Update password?" are as built (rows 42 to 47), and are **the person's**: a sign-in an agent typed through a Vault grant is not offered for saving again, and an agent-driven sign-in never opens the band for the agent to answer. What this spec adds:
- **"Add a username"** when a sign-in was captured with none (row 50).
- **Mirroring** a later edit of a made password into its confirmation field (row 53).
- **Two more submit signals** (row 46): a successful `fetch` or XHR that follows the click, and a DOM change after a fill, with the HTTP-error-page check. They run only when the form was not removed or navigated away, as in Chrome, so a plain page does not change.

## P2.6 Import

The Vault already takes Chrome's CSV export and 20+ other sources (#7265, #7298), so a person with sign-ins in Chrome can see them in the list as soon as they import. Two open questions for the operator are in P2.10 (the import prompt on first use, and whether to read Chrome's own store directly).

## P2.7 Defaults and settings

"Saved sign-ins" group in Settings > Browser (P2):
- **Offer to fill from the Vault** (on). Off hides the list and leaves the key button.
- **Offer strong passwords** (on).
- **Declined sites** (the "Never for this site" list, per profile; remove one to be asked again: row 45).
- **Ask the device before filling** (off; Windows and macOS only: row 57). This one is P3.

Nothing here weakens a rule of P2.4; none of these can be switched on by an agent without the person's own act.

## P2.8 Phases

| Phase | What ships | Done when |
|---|---|---|
| **P1** | The list under a field on click or arrow key, the typed-text filter, the order, the footer, the keyboard, "Use a strong password", exact origin, the P2.4 rules, R2 (or R4 on a platform where R2 cannot be done) | A person clicks a password field on a login page, picks a sign-in with the keyboard and with the mouse, and signs in; an agent's screenshot and snapshot of the same moment show no list and no value |
| **P2** | The Settings group, the declined-sites list, "Add a username", mirroring, the extra submit signals | The Settings page controls each behave; a declined site is not offered again |
| **P3** | "Ask the device before filling" (Windows Hello, macOS), off by default | With it on, a pick asks the OS first; Linux shows no such control |
| **P4** | Public-suffix and "also use on" rows with the other site's host as a label (rows 33 to 36), fill on the first gesture as an opt-in (row 1), the list over an autofocused field (row 5), the manual fallback (row 48), the four-assumption check (row 23) | Each is a separate small PR |

**As built, P1 (the pane-drawn route R4).** No run has confirmed a host-drawn popup on any platform, so P1 ships R4 on every path, as P2.2 says. It is the list of P2.3 drawn by the pane's own UI under the nav bar's key button, and a later phase can move it under the field with no change to what srv, the listener and the rules below it do.
- **Page side** (`crates/cef/src/browser_pane/signin_notice.js`): the person's own click, or Up or Down, in a login field reports `open` with the kind of field (`username`, `password` or `newpassword`); a key the list takes reports `key`; a press elsewhere reports `close`. Only trusted events count. The listener sets no focus and draws nothing, and swallows Up, Down and Escape only while srv says a list is showing and Enter only with a row selected (Tab picks and still moves focus). **One thing differs from "no page text" in P2.3:** a user name field's report carries what it holds, clipped to 64 characters, because the list is filtered by it. A password field's value is never read there, and the text is kept nowhere.
- **The agent rule (P2.4 item 1):** `browser_pane/agent_input.rs` marks a pane while any `Input.*` call of an agent (`CdpSession::call`) is outstanding and for 1.5 s after; the host drops every list report in that time, so an agent's click or key neither opens, moves nor picks. The person's input never passes through `CdpSession`.
- **srv** (`server/browser_signin_list.rs`): the rows are the tab's profile's sign-ins for the frame's own origin (`https`, or `http` on this computer; never in an Incognito tab), most recently used first and then by user name, filtered by the typed text as a case-insensitive prefix on a user name field; the selected item moves with Up and Down and wraps. The pane is told **masked** user names only (`browser:signin-list`). An answer to every report tells the host two booleans (a list is showing, a row is selected), which it sets in the listener's own world.
- **The list** (`browser-signin-list.tsx`): the kit's popover, marked operator-only, never taking focus, with the strong-password row for a field for a new password and a "Manage passwords" row that opens the Vault on its logins. A click on a row is `browser.signin_fill` or `browser.signup_password`, the person's pick as for the key button.
- **Not in P1:** moving the list under the field (R2), Page Up and Page Down, closing on a scroll, filtering a footer-only list differently, full user names (decision 1 stays "masked until R2's item c is confirmed").

## P2.9 Not in scope

Passkeys and WebAuthn (rows 20, 64); federated sign-ins (row 41); the Credential Management API and "sign in automatically" (rows 19, 69); leak and password-checkup checks that need a service (row 62); sync between devices (row 68); Chrome's own store as a source of live data; enterprise policy. A weak or reused check that runs locally is a separate spec.

## P2.10 Decisions for the operator

1. **Usernames in the list: shown in full, or masked?** Chrome shows them in full (row 10). Today's key menu shows a masked form (`al***`) because an agent can see that menu's pane. A host-drawn list that no agent or page can see could show them in full, and the person recognises their accounts at a glance. **Recommend: in full, as long as R2's item c (not in an agent's screenshot) is confirmed; masked until then.**
2. **A single saved sign-in: wait for the pick (as built) or fill on the first click like Chrome?** Chrome fills before the person does anything (row 1). Here that would put the value in the page for any script on it to read. **Recommend: wait for the pick; offer the fill-on-first-gesture as an opt-in in P4.**
3. **Public-suffix matches (row 33):** worth the extra risk in P4, or leave exact-only? **Recommend: P4, behind a label and only after a pick, as Chrome.**
4. **Reading Chrome's own saved passwords directly** (not a CSV): the importer asks the person once and reads the store, which needs the OS key. Chrome's current store may need a Chrome-version-specific key route. **Recommend: no; the CSV import stays the route.**
5. **The engine route.** R2, with R4 as the per-platform fallback, once Korp confirms a to d. If Korp says R2 is not feasible at all, the choice is R4 only, and the operator should know the list would then sit in the pane's chrome rather than under the field.

## P2.11 Tests, and checking it in a task dev build

- **Rust** (host): matching returns exact-origin rows only, in row 9's order; the filter; a frame is matched by its own origin and not the page's word; a container profile never returns the person's sign-ins; no row or value appears in any Browser tool answer.
- **Listener** (vitest and the parity test): the field roles and the four-assumption check; the listener reports a rectangle and a handle, never a value.
- **Frontend** (the host-drawn surface): keyboard (Up, Down, Page Up, Page Down, Enter, Tab, Escape), the order and the footer, no list when only the footer would show, light and dark themes, narrow panes.
- **A task dev build, by the author, on the author's own isolated build and never the operator's instance:** a local test site with a username-first flow, a one-page login, a sign-up, an iframe from another origin and a page that dispatches its own focus and click events; confirm the list does not open for the page's events or an agent's click, does open for the person's, and an agent's screenshot and snapshot never show it. Visual changes also need the macOS (Masty, starpower) and Linux (Parlo, charlie) look checks before a merge.

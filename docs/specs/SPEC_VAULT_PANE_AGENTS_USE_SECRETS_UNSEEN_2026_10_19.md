# SPEC: the Vault: one pane for the person's secrets, which agents use without seeing them

**Status:** active — P1 (§4, §11) shipped in #5180; P2a (§5: the pane, the store, the `Vault` tool, agents kept out) P2b (Reveal and Copy, Windows) and P2c (`.vault` export and import) are built; P3 to P8 remain. The operator took the recommended answers to §13 on 2026-10-19.
**Date:** 2026-10-19
**Author:** Agent2 #6 (agent2-6@narko), at the operator's request
**Trigger (operator, 2026-10-19):** an agent filling a US state's corporate-registry certificate request form was refused on the address, city, ZIP and email fields, then could not tick a checkbox, so the person had to finish the form by hand. "Agentmux needs to be able to fill in personal information including ssn." … "we'll need to pivot into that work now, write a spec to file." On a first draft that put saved values in Settings: "sounds like we may need a new pane type, a vault, or settings wouldn't really make sense" … "api keys and anything secret can be kept there." … "we want rich features, like support for pw protected encrypted export file (.vault file), and imports from kdbx files and last pass, and any other popular services should be supported."
**Builds on:** `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` (§5.1 the secret-field guard, §5.4 approvals, §5.6 host-only answers), `SPEC_BROWSER_PANE_FORMS_IN_FRAMES_AND_VISUAL_FALLBACK_2026_10_16.md` (§5.1: SSN/TIN fields made secret, screenshots black out secret fields), `SPEC_BROWSER_PANE_BYPASS_2026_10_16.md` (§4.3: the secret guard is "never lifted"), `PLAN_BROWSER_PANE_FORM_FILLING_FIXES_2026_10_17.md` (§4.2, §6).
**Changes:** the "agent never types" rule of the base spec §5.1 and the "never lifted" rule of the bypass spec §4.3, as §10 says.

---

## 1. Summary

1. **Fix the false positives first (P1, §4).** Any field whose `autocomplete` contains `new-password` is secret today. Sites put that on ordinary fields to stop the browser's autofill: on the triggering form, the address, city, ZIP and email fields. After P1 it counts only on a real password field. A styled checkbox whose input is hidden gets a reference through its label (§11).
2. **A new pane type, the Vault (`view: "vault"`, §5).** One place for everything secret the person owns: personal data (SSN, date of birth, passport and licence numbers), bank accounts and payment cards, API keys and tokens, site logins, and free-form secret notes. Values are encrypted with a Vault key that lives in the OS keychain through the existing `secret_store` (§5.5). Only the person adds, edits, reveals or deletes, in the pane.
3. **Agents use an entry by name and never see its value (§6, §7).**
   - In a browser pane: `Browser` `fill` with `vault: "<entry>"`. The host types the value; the result, snapshots, screenshots, logs and transcripts show only that the entry was filled.
   - In a command: `Terminal` `run` with `secrets: {"STRIPE_API_KEY": "<entry>"}`. srv puts the value in that one process's environment and scrubs it from the output.
4. **The person decides where each entry goes.** The first use of an entry on a site, or by an agent for commands, asks in a banner answered through the host, never in chat. "Always" covers that site, or that agent, and nothing wider.
5. **No value yet?** The banner has a field where the person types it once, with **Save to Vault** (§6.5). The agent can also ask for a new entry with `Vault` `request` (§5.4).
6. **Import and export (§8).** Export to a password-protected, encrypted `.vault` file (an age file the person can also open without AgentMux) and to KeePass `.kdbx`. Import from `.vault`, KeePass `.kdbx`, LastPass, 1Password, Bitwarden, Proton Pass, Dashlane, Keeper, NordPass, RoboForm, Enpass, Zoho Vault, the browsers' password exports, `.env` files and any CSV. Only the person imports or exports; no agent route reaches either.
7. **Not done:** agents typing a secret they hold themselves (§9.2, §13 question 1), and agents filling passwords and one-time codes (§13 question 2). Passwords can be stored in the Vault; only the filling waits for that decision.

## 2. What exists (verified on `main` at `cc8abe254`, 2026-10-19)

| Piece | Where | What it does |
|---|---|---|
| Secret rule | `crates/cef/src/browser_api/snapshot.rs:144-177` (`is_secret`) | Secret if `type=password`; or **any** `autocomplete` token `current-password`, `new-password`, `one-time-code`, `cc-*`; or a name, id, aria-label, placeholder or label naming an SSN/ITIN/TIN; or a name or id like password, otp, cvc/cvv, card number |
| Snapshot mark | `snapshot.rs:559-560` | `[secret: ask the user to fill this]`; the value is never shown |
| Fill and typing refusals | `crates/cef/src/browser_api/act.rs:901-927`, `act.rs:554-579`, text at `act.rs:581` | `fill` and `key` refuse a secret field, before and after focusing it |
| Masking | `act.rs:1051` (`redact_secret`); `crates/cef/src/browser_api/routes.rs:392` (#5089) | Read-backs never return a secret value; screenshots black out secret fields |
| Action log | `crates/srv/src/server/browser_actions.rs:13` | A fill names its field, never its value |
| Keychain store | `crates/srv/src/identity/secret_store.rs` | OS keychain, or a `0600` file store when headless; `Zeroizing` reads |
| Secrets already stored | `SecretRef::Keychain` (provider account API keys, OAuth, MuxBus), `crates/srv/src/identity/browser_credential_store.rs` (browser Basic-Auth logins) | Managed in the Identity and Accounts panes and the browser's auth prompt; there is no single view of them |
| Command env | `crates/srv/src/server/http_shell.rs:93-118` (`POST /api/v1/shell/create`) | A command runs with srv's env, the agent's `cmd:env`, then the caller's `env`; `gh_guard` applied last |
| Pane types | `frontend/app/block/block-registry.test.ts:18-22`; `frontend/app/view/<view>/`; `openOrFocusPaneByView` (e.g. `frontend/app/view/warden/open-warden.ts`) | Memory, Identity, Warden and the others are views with their own folder and a registry manifest |
| Driven-pane limits | bypass spec §4.1 | `eval`, UI click by selector, and `key` Enter/Space are refused on a pane an agent opened |

## 3. Goals and non-goals

**Goals**
- An agent fills every ordinary field of a form itself: name, address, email, phone, company details.
- An agent fills SSNs, card and bank numbers and similar from the Vault, and runs commands that need an API key from the Vault, with the person deciding where each entry may go.
- No Vault value enters any agent's context, transcript, tool result or log.
- One pane where the person sees and manages every secret AgentMux keeps.
- A secret manager good enough to move into from another one and out of again: encrypted export, and import from every popular service (§8).

**Non-goals**
- Agents filling passwords and one-time codes, and solving CAPTCHAs (§13 question 2).
- Syncing the Vault through the cloud relay or to other machines. It is local to this AgentMux; the `.vault` file is how it moves.
- A browser extension or autofill outside AgentMux's own panes.

## 4. P1: fewer false positives

### 4.1 The `autocomplete` rule

Today any `autocomplete` token `new-password` or `current-password` makes a field secret (`snapshot.rs:149-151`). Sites use `autocomplete="new-password"` on ordinary fields because browsers ignore `autocomplete="off"`. On the triggering form, `mailingAddress1`, `city`, `postalCode` and `email` all carry it.

New rule: a password token makes a field secret only if one of these is also true:
- `type=password`;
- its name, id, aria-label, placeholder or label names a password, passphrase, passcode or PIN (`pin`, `pw` and `pwd` as whole words).

A "show password" button flips a field's type to `text`; such a field is still caught by its label or name, which say password. (Built in P1, `crates/cef/src/browser_api/secret.rs`, with the rule split out of `snapshot.rs`; the approval banner's copy of the rule in `act.rs`'s `COMMIT_INFO` follows it.)

`one-time-code` and `cc-*` still make a field secret on their own: sites don't use them to block autofill.

### 4.2 Say why

The refusal and the snapshot mark name the rule that fired (`[secret: ask the user to fill this; its name or label says Social Security or taxpayer ID number]`, "… field (it is a password field): AgentMux never lets an agent type into one"), so a false positive is easy to report and the agent can tell the person exactly what to type.

### 4.3 Tests

- `input type=text autocomplete=new-password name=city` → not secret; the same with `name=email` → not secret.
- `type=password autocomplete=new-password` → secret; `type=text autocomplete=new-password name=newPassword` → secret; `name=pin` → secret.
- `type=text autocomplete=current-password` labelled "Password" (a shown password) → still secret.
- Fixture: a copy of the triggering form's submitter block; an agent fills every field.

## 5. The Vault pane

### 5.1 Why its own pane

Settings holds how AgentMux behaves; the Vault holds things the person owns, with its own reveal, approval and audit rules. Identity holds the accounts agents run as; Warden watches what agents do. A Vault entry belongs to the person and is lent to agents one use at a time. A pane of its own also lets it stay closed and locked until the person opens it.

### 5.2 Layout

A new view, `vault`, with a manifest in the block registry, a `lock` icon, opened from the widget bar, the command palette and `openOrFocusPaneByView("vault")`. Like Warden, a rail of sections:

| Section | Entry kinds | Fields of an entry |
|---|---|---|
| **Personal data** | `ssn`, `itin`, `tin`, `ein`, `date-of-birth`, `passport`, `drivers-licence`, `other-id` | label, value, owner (whose it is) |
| **Cards and banks** | `card` (number, expiry, CVC, name on card), `bank-account` (account, routing) | label, parts |
| **API keys and tokens** | `api-key` | label, service, value, suggested env name (`STRIPE_API_KEY`), expiry, notes |
| **Logins** | `login` | label, site, username, password, one-time-code seed (stored only, §13 question 2) |
| **Notes** | `note` | label, secret text |
| **Kept by AgentMux** | read-only list of what `secret_store` already holds (provider account keys, MuxBus, browser Basic-Auth logins) | label, where it is managed; a link to that pane. No values (§13 question 5) |
| **Activity** | — | every use: entry, agent, where (origin or command), when, allowed or denied |

Each entry also shows its **grants** (§6.4, §7.3) with a revoke button, and its last use.

### 5.3 Locked, revealed, captured

- Values are hidden in the pane: it shows each entry's label, plain fields and the main value's last characters (§5.5), never a value. Secret parts are typed into hidden inputs and never sent back to the pane; changing an entry replaces only the parts the person types.
- **Reveal** and **Copy** (built in P2b) ask the OS to confirm the person each time, through the host: Windows Hello (`UserConsentVerifier`: PIN, fingerprint or face). Touch ID and the desktop's polkit prompt aren't built; there Reveal and Copy are disabled and say so. Nothing is unlocked for a session, so there is nothing to relock: a confirmation covers one Reveal or Copy.
  - **Copy** never brings the value into the page: the host confirms the person, fetches the value from srv as the host, writes it to the clipboard and clears it 30 seconds later if the clipboard still holds it. On Windows it is marked to stay out of the clipboard history and the cloud clipboard (`CanIncludeInClipboardHistory`, `CanUploadToCloudClipboard`) and out of clipboard monitors (`ExcludeClipboardContentFromMonitorProcessing`); if marking fails the copy is cleared and refused.
  - **Reveal** returns the value to the pane, which shows it for 15 seconds and hides it on **Hide**, when the window loses focus and when the dialog closes. Losing focus while a Reveal or Copy is waiting is ignored: the Windows Hello prompt takes focus from the window, and that is expected.
  - The host's `vault_op` relay refuses the `reveal` operation, so a value only comes through `vault_reveal` or `vault_copy`, which confirm the person first. Each use is logged in Activity (who, which entry and part, never the value) and noted on the entry as its last use; a copy is logged once the host reports the clipboard has it (or that it failed), so the trail never says copied when nothing was.
  - If Windows Hello isn't set up, the error says how to set it up. The account-password fallback listed in §5.7 isn't built yet.
- **Agents can't capture it** (built in P2a):
  - `UI` `query` and `click` already reach only the caller's own pane, and the Vault's forms and confirmations are drawn inside the Vault pane, so neither reaches it. `UI` `keys` sends only keys from the shortcut table, as key events that type no text.
  - `UI` `screenshot` is clipped to the caller's own pane. A Vault pane magnified or floating over it would be inside the clip, so the host finds every element marked `data-agent-hidden` (the Vault's root) inside the clip just before and just after the capture, and srv blacks those boxes out as it does secret fields (#5089). If the page can't be asked, the screenshot is refused.
  - `UI` `window` grabs the OS window itself, outside srv and the host, so it can't black out one pane. Instead, while a Vault pane is on screen in any AgentMux on the computer, it refuses to capture any AgentMux window. Each Vault pane notes that it's showing every 10 seconds through the host, one note at a time so they arrive in order; a note lasts 30 seconds and is cleared when that pane is hidden or closed. Each pane has its own note, so one pane hiding doesn't clear another's. The notes are kept in the identity store every instance shares, and only the host can write one, so an agent can't clear it. If srv can't be asked, the capture is refused.
- No MuxAPI route returns a Vault value.

### 5.4 Who writes it

Only the person, in the pane or in a banner (§6.5). Writes go through the host, as approval answers do (base spec §5.6); srv refuses Vault writes on every agent route, so a tricked agent can't plant or replace a value.

Agents get a `Vault` MuxAPI tool:
- `list`: `[{id, label, kind, section}]`. No values, tails, fields or grants. It reads no value, so it never touches the keychain.
- `request {kind, label, why}`: every AgentMux window shows a prompt naming the agent (by its signed identity; an unsigned request says "not a verified agent"), the kind, the label and the reason, and the Vault lists the request at its top. **Fill it in** opens the Vault on a new entry of that kind; **Dismiss** declines it. The agent gets back the new entry's id, `declined`, or `pending` when its wait (default 5 minutes, at most 10) ends first. A request stays open for an hour, so the person can still answer, and the agent then finds the entry with `list`.

An entry's id is made once from its kind and label (`ssn-alex`, then `ssn-alex-2`) and never changes, so an agent's reference survives a rename. It has no `.`, which names a part (`card-company.number`, §6.1).

### 5.5 Storage

- **One Vault key in `secret_store`** (the keychain, or the headless file store), not one item per value: Windows' Credential Manager caps an item at 1280 characters, which a note or a long key passes, and macOS asks the person about each item separately. The key is 256 random bits, made with the first entry. If it goes missing while entries exist, srv says so and makes no new key, which couldn't open them.
- **Each entry is one row** in the identity store (`db_vault_entries`), shared by every AgentMux on the computer like the keychain. The row holds the id, kind, label, plain fields and tags, and the secret parts, sealed together: ChaCha20-Poly1305 (`ring`), a random nonce each time, and the entry id as associated data, so sealed parts moved to another row don't open. No column holds a value. One row per entry also lets an import land in one transaction (§8.5).
- The main value's last characters (4 of a long value, 2 of a short one; none of a value of 4 characters or fewer, a password, a date of birth or a note) are worked out for the pane when it lists entries; they aren't stored.
- **Activity** (`db_vault_activity`) records what was done, by whom, to which entry and where; never a value.
- Deleting an entry deletes its row, and its sealed parts with it.

### 5.6 Activity and grants in Warden too

The operator decided on 2026-10-19 that the Vault stays its own pane, and that Warden also shows its oversight half: the record of agents' uses of entries and the standing "Always" grants, as a section or as Audit events, linked both ways with the Vault. Built with the grants, in P3.

### 5.7 What the person is asked, and when

The operator asked on 2026-10-20 that people know before they're asked. The Vault pane opens with a note saying, for this OS, what may ask the person and when (dismissed with **Got it**, reopened from the header's ⓘ), and the first save, which creates the Vault key, says what is about to ask. Nothing in the Vault asks for administrator rights (UAC, sudo), and no agent sees or answers any of these prompts. Each step that adds a prompt adds its row here and its line to the note (`frontend/app/view/vault/vault-prompts.ts`).

| When | Windows | macOS | Linux | Since |
|---|---|---|---|---|
| Saving, changing or listing entries | nothing | the first time AgentMux uses the Vault key, and again after AgentMux updates: allow AgentMux to use the keychain item (**Always Allow**; Deny leaves the entries unopenable) | if the keyring is locked: its password, once | P2a |
| Reveal or Copy | Windows Hello (PIN, fingerprint or face); needs Hello set up (an account-password fallback is not built) | Touch ID or the account password (not built; Reveal and Copy are disabled) | the desktop's polkit prompt (not built; disabled) | P2b |
| An agent asks for an entry | AgentMux's own prompt in every window: Fill it in, or Not now | the same | the same | P2a |
| Importing an encrypted file | the file's own password, typed in the pane | the same | the same | P2c, P5 |

## 6. Agents fill browser fields from the Vault

### 6.1 The action

`Browser` `fill` takes `vault: "<id>"` (or `"<id>.<part>"`, e.g. `card-company.number`) instead of `text`, on secret and ordinary fields alike. A card can fill a whole payment form: each part goes to the field whose `autocomplete` says `cc-number`, `cc-exp`, `cc-csc`, `cc-name`.

### 6.2 What the agent gets back

`{"filled_from_vault": "ssn-alex", "value": "[secret: hidden]"}`, plus the page's validation message if it rejected the value. Length and mask warnings say only "the page shortened or reformatted it".

The field is **tainted** from then on: srv marks its backend node id secret until navigation, so a snapshot, `find`, read-back or screenshot of it is masked even if it isn't a secret field. A tricked agent can't fill the SSN into a visible comment box and read it back.

### 6.3 Where a value may go

Before typing, srv checks:
1. **The site:** the origin of the field's own frame (frames spec §5.3), not only the pane's top origin.
2. **The kind:** does the field look like the entry's kind? `ssn` matches a field whose markup or label names an SSN or taxpayer ID (`names_a_tax_id`); a card part matches its `cc-*` token or a name like `cardnumber`; `api-key` matches a field named or labelled key, token or secret; and so on.

| Site already granted for this entry | Kind matches | What happens |
|---|---|---|
| yes | yes | Fills with no question; Activity records it |
| yes | no | Asks, saying the field doesn't look like the entry's kind |
| no | either | Asks |

### 6.4 The banner

It shows the entry's label, the field's label, the frame's origin and a screenshot: **[Allow once] [Always on <origin>] [Deny]**. **Always** is offered only when the kind matches, and is stored as a grant on the entry. Auto-approve (the bypass spec) doesn't answer these; it covers committing clicks, not where secrets go. Answers come only from the person through the host (`browser_attention_resolve`); an agent holding srv's auth key gets 403.

### 6.5 No entry yet: the person types it in the banner

When the agent fills with a `vault` id that doesn't exist, or uses `Browser` `request_value {ref, kind, label}`, the banner asks for the value in a field that belongs to AgentMux, not the page:

> Agent2 #6 wants to fill *Social Security Number* on **irs.gov**.
> [ value field ]  ☐ Save to Vault as "Social Security number"
> [Fill] [Cancel]

The host types it into the page. With the box ticked, it is also saved, with a grant for this origin. This replaces most hand-offs for forms.

### 6.6 The path of a value

Keychain → srv (`Zeroizing`) → host over host IPC → `Input.insertText` in the page. Never MCP, a tool result or srv's ordinary API responses. The host log records entry id, field and origin, never the value.

## 7. Agents run commands with a Vault entry

### 7.1 The action

`Terminal` `run` (foreground or `background: true`) takes `secrets: {"<ENV_NAME>": "<id>"}`. srv adds each value to that one process's environment in `POST /api/v1/shell/create`, after `cmd:env` and the caller's `env` (`http_shell.rs:93-118`), before `gh_guard`. Interactive shells (`Terminal` `open`/`type`) don't take secrets: a value would stay in a shell the agent keeps typing into.

### 7.2 Output scrubbing

Before output reaches the agent, the pane or a log, srv replaces each injected value with `[vault:<id>]`, and also its base64, URL-encoded and hex forms. **This is a guard, not a boundary.** A command the agent writes can transform the value and print it, or send it anywhere the key reaches. What limits that is §7.3: the person sees the command before an entry is first used, and should prefer keys scoped to what the task needs.

### 7.3 Approval

The first use of an entry by an agent asks in a banner on the agent's pane: the entry, the agent, the command line and its working directory, **[Allow once] [Always for <agent>] [Deny]**. "Always" is a grant per entry and agent, shown and revocable in the Vault. Answers come through the host, as in §6.4.

### 7.4 Where commands can run

Host commands first. `connection` to WSL or SSH is refused with a Vault secret until a later phase decides how a value crosses to that host (SSH `SendEnv` needs the server's `AcceptEnv`; writing it to a remote file is worse).

## 8. Import and export

The Vault is a full secret manager, not only a store for agents, so the person can move secrets in and out of it.

### 8.1 Who can

Only the person, from the Vault pane (**Import…** and **Export…** in its header). Files are chosen in the host's native file dialog. No MuxAPI route starts an import or export or reads its file, so an agent can't export the Vault to a file it then reads. Every import and export is recorded in Activity: when, which file name, how many entries, no values.

### 8.2 Export: the password-protected `.vault` file

- **What the person picks:** all entries, some sections, or chosen entries; a passphrase, typed twice, with a strength meter and a **Suggest one** button (a random word passphrase); where to save it.
- **What it holds:** each entry's label, kind, section, owner, notes, every value and one-time-code seed. Not grants, Activity or the "Kept by AgentMux" items, which belong to this install.
- **Format, version 1:** an [age](https://age-encryption.org/v1) file encrypted to the passphrase (age's scrypt recipient), whose plaintext is UTF-8 JSON in the schema documented in `docs/api/VAULT_FILE_FORMAT.md` (`{"format": "agentmux-vault", "version": 1, "exported_at", "app_version", "entries": [...]}`). The extension is `.vault`.
  - *Why age:* a reviewed, standard format with authenticated encryption and a password-hardening step, so AgentMux adds no cryptography of its own, and the person can open their own export without AgentMux (`age -d backup.vault > vault.json`). The Rust `age` crate is maintained and permissively licensed. (§13 question 6 has the alternative.)
  - Size cap 64 MiB. Plaintext is held in `Zeroizing` buffers and never written to disk. The file is written beside its destination and renamed, so it appears whole or not at all. Opening a file refuses scrypt work factors above 22.
  - **Built in P2c.** The export dialog lists the entries by section (all selected), asks for the passphrase twice with a strength meter and **Suggest one** (eight random words from a 256-word list: 64 bits, which scrypt slows down), then the host asks where to save. A passphrase is at least 8 characters.
- **Reminders:** the pane shows when the Vault was last exported and suggests one after many changes, since keychain items don't move to a new machine by themselves.

### 8.3 Export to other formats

- **KeePass `.kdbx` (KDBX 4)**, protected by a passphrase and optionally a key file, so the person can open their Vault in KeePassXC, KeePassDX or Strongbox. Kinds that KeePass has no type for become entries with custom fields named after them (`SSN`, `Card number`, …).
- **Plain CSV or JSON:** only behind a warning that the file will hold every secret unencrypted (§13 question 7).

### 8.4 Import: sources

The pane's import wizard: pick the source, then the file; a password or key file if the file is encrypted; a preview; import. Each importer is checked against a real export from the current version of its service before it ships, and has fixtures in the tests, because services change their formats.

| Source | What the person exports from it | Encrypted? | What comes in |
|---|---|---|---|
| **AgentMux** | `.vault` (§8.2) | yes, passphrase | everything |
| **KeePass, KeePassXC, KeePassDX, KeeWeb, Strongbox** | `.kdbx` (KDBX 3.1 and 4; AES or ChaCha20; AES-KDF or Argon2) | yes, master password and/or key file | logins with TOTP, notes; custom fields named like a kind (SSN, card number, API key) become that kind |
| **LastPass** | CSV export (`url, username, password, totp, extra, name, grouping, fav`) | no | logins with TOTP; secure notes (`url` = `http://sn`) by their `NoteType:` in `extra`: Credit Card, Bank Account, Social Security, Driver's License, Passport and Address map to their kinds, SSH Key and Server to notes with their fields; the rest become notes |
| **1Password** | `.1pux` (a zip of JSON) or CSV | no | logins, cards, bank accounts, identities, SSN, driver licence and passport items, API credentials, secure notes, servers and databases |
| **Bitwarden, Vaultwarden** | JSON, password-protected JSON, or CSV | the password-protected JSON, with its password | logins with TOTP, cards, identities (SSN, passport, licence), secure notes, SSH keys |
| **Proton Pass** | zip of JSON, optionally PGP-encrypted | optional, passphrase | logins, cards, identities, notes, aliases as logins |
| **Dashlane** | CSV zip | no | credentials, payments, IDs, personal info, secure notes |
| **Keeper** | JSON or CSV | no | records by type, custom fields by name |
| **NordPass, RoboForm, Enpass, Zoho Vault, Sticky Password** | CSV (Enpass: JSON) | no | logins, cards, notes, identities where present |
| **Browsers:** Chrome, Edge, Brave, Firefox, Safari, Apple Passwords | password CSV | no | logins |
| **`.env` files and shell `export` lines** | the file itself | no | API keys, with the variable name as the suggested env name (§5.2) |
| **Any other CSV** | the file | no | a column-mapping step: the person says which column is label, value, site, username, kind |

Attachments, passkeys and file items can't be imported; the preview lists how many were skipped and why.

### 8.5 Import: the preview

Built in P2c for `.vault` files: the host asks which file, srv decrypts it into memory (a wrong or missing passphrase writes nothing), and the dialog shows counts per section, the first five entries of each, which are duplicates, and what can't come in with the reason. Nothing is written until **Import**, which writes all of it in one transaction or none of it. What srv holds waits ten minutes and is dropped when the dialog closes. An entry keeps its id from the file when it is free.


- Counts per Vault section, and the first entries of each, values hidden.
- **Duplicates:** an incoming entry with the same site and username (logins), or the same kind and value (others), as an existing one is marked. The person chooses per import: skip, replace, or keep both.
- Folder and group names become tags, so nothing about how the person organised them is lost.
- Nothing is written until **Import** is pressed. Then each value goes to the keychain and the rest to the database in one step; a failure part-way removes what was written.

### 8.6 Plaintext export files

Most services export plaintext CSV or JSON: every secret readable by anything that reads the disk. After an import from one, the pane says so and offers **Delete the file**. It warns when the file is in a synced folder (OneDrive, Dropbox, iCloud Drive, Google Drive), where a copy may already be in the cloud and in the service's file history. It doesn't promise secure erasure: on SSDs an overwrite doesn't reliably reach the old blocks.

### 8.7 Parsers are attack surface

An import file can be crafted. Parsers run in srv with size caps (64 MiB file, 100 000 entries, 1 MiB per value) and limits on zip expansion, and are fuzzed (`cargo fuzz` targets for KDBX, the `.1pux` and Proton zips, JSON and CSV). Argon2 parameters in a `.kdbx` file are capped (memory 1 GiB, iterations 100) so a file can't hang srv; a file over the cap is refused with that reason.

## 9. Security

### 9.1 What changes

Before: no agent could put a sensitive value into a page, and API keys an agent needed had to be pasted into its chat or a file. After: an agent directs a stored value to a field or a command, and the value still doesn't reach it. What remains is a value going to the wrong place: a phishing page, the wrong field, or a command that leaks it. §6.3 and §7.3 put the first use of every entry in front of the person, with the exact site, field or command, and "Always" never covers more than that site or that agent.

### 9.2 Why not let the agent type it (the alternative not chosen)

The simplest change is to lift the guard and let the agent `fill` an SSN as text, or paste a key into its command. Then the value is in the model's context, and so:
- it goes to the model provider with every later turn;
- it is written to the provider's local transcript (for Claude Code, `~/.claude/projects/**/*.jsonl`) and AgentMux's conversation history, continuation summaries and `search_history`;
- it is readable by **any other agent on the host** through `Agents` `transcript` and `conversations`;
- the agent can repeat it into a chat message, a PR or a jekt, by mistake or because a page asked it to.

The Vault gets the same work done with none of these. §13 question 1 asks whether to also offer agent typing as an opt-in.

### 9.3 What stays

- `eval`, UI click by selector and `key` Enter/Space stay refused on driven panes (bypass spec §4.1): each would let page content or an agent reach a value without §6.3.
- Screenshot blackout covers secret fields, tainted fields and Vault panes.
- Same OS user: a process running as the user can read the keychain or drive CEF directly. That is the residual the rest of MuxAPI already accepts.
- Headless srv's file store protects Vault values by file permissions only, as it does account keys today.

## 10. Amendments to earlier specs

- `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` §5.1: an agent still never types a secret value it holds, but it can fill one from the Vault (this spec §6).
- `SPEC_BROWSER_PANE_BYPASS_2026_10_16.md` §4.3: unchanged in effect. Auto-approve still doesn't let an agent type a secret; Vault fills have their own approval (§6.4), which auto-approve doesn't answer.
- `PLAN_BROWSER_PANE_FORM_FILLING_FIXES_2026_10_17.md` §6 ("the operator types secrets; the agent checks the result"): §6.5 is that pattern in one step instead of a hand-off.
- The `Browser` tool description and the `[secret: …]` mark: "fill it from the Vault (`Vault` `list`), or ask the person with `request_value`".

## 11. Also found in the same session: a styled checkbox can't be ticked

The form's "Short Form Good Standing" checkbox is the common Bootstrap pattern: `<label><input type=checkbox> <span class="cr">…</span> <b>Short Form Good Standing</b></label>`, with the input `display: none` and the span drawn as the box. The accessibility tree leaves the input out, so the snapshot showed only the table cell and the info link beside the label, and there was no reference for `check`; clicking the info link opened its help; UI click by selector and `key` Space are refused on driven panes. The agent had no way to tick it.

Fix (built in P1):
- The snapshot, `find` and every frame look for checkbox and radio inputs with no box whose label has one (`act.rs` `hidden_choices`, at most 200 a frame), and add each as a `checkbox` or `radio` node named by its label, first inside the label's node, carrying the input's backend id (`snapshot.rs` `add_hidden_choices`): `checkbox "Short Form Good Standing" [ref=e112] [checked=false]`.
- `check` and `click` on it click the label: `CLICK_POINT` picks the first point in the label (its centre, then its children's) where the label itself is hit and not a link or button inside it. `check` then reads the input's state back as before.
- Tests: a hidden checkbox inside its label gets a line and the input's reference, before the link beside it; one whose label isn't in the tree is still listed, at the end.

## 12. Phases

| Phase | What | Done when |
|---|---|---|
| **P1** | §4 (autocomplete rule, reasons) and §11 (hidden checkboxes). Small, independent, ships first | The triggering form's submitter block and checkbox are filled by the agent with no hand-off |
| **P2a** | The Vault pane and store (§5): sections, add, change and delete, Activity, "Kept by AgentMux"; agents kept out of it (§5.3); the `Vault` tool's `list` and `request` | The person keeps an SSN, a card and an API key in the Vault; an agent asks for one and gets its id; no agent route returns a value, a tail or a field |
| **P2b** | Reveal and Copy with OS confirmation and the clipboard clear (§5.3), built for Windows Hello; the macOS and Linux confirmations follow | Reveal asks Windows Hello; a copied value is gone from the clipboard 30 s later |
| **P2c** | `.vault` export and import (§8.1, §8.2, §8.5), so there is a backup early | The person exports the Vault, deletes the entries and imports them back |
| **P3** | Browser fill from the Vault (§6.1 to §6.4, §6.6), taint, grants; Activity and grants in Warden (§5.6) | An agent fills an SSN fixture field; its result, snapshot, screenshot and transcript have no trace of the value; another origin is asked |
| **P4** | §6.5: the banner's value field and **Save to Vault**; `request_value` | A form with an SSN field is done by the agent plus one value typed in the banner |
| **P5** | Importers, first wave (§8.4): KeePass `.kdbx`, LastPass, 1Password, Bitwarden, the browsers' CSVs, `.env`, any CSV; plaintext-file handling (§8.6); fuzzing (§8.7); KeePass export (§8.3) | Each imports its real-export fixture with the right kinds and no loss the preview didn't list |
| **P6** | Importers, second wave: Proton Pass, Dashlane, Keeper, NordPass, RoboForm, Enpass, Zoho Vault, Sticky Password | The same, per source |
| **P7** | §7: `Terminal` `run` with `secrets`, scrubbing, approval, grants | A command reads `STRIPE_API_KEY` from the Vault; its echoed output shows `[vault:<id>]` |
| **P8** | Only on a yes to §13 question 1 or 2 | — |

## 13. Decisions for the operator

**Answered 2026-10-19:** the operator went with the recommended answers: 1 no; 2 store passwords, don't fill them yet; 3 the first use per site (forms) and per agent (commands) asks, with "Always" for that site or agent; 4 OS confirmation (Windows Hello, Touch ID) for Reveal and Copy; 5 list AgentMux's own secrets read-only; 6 age; 7 offer plaintext export behind a warning; 8 yes.

1. **Agent-typed secrets.** Should an agent also be able to type an SSN, card number or key it holds itself, after the person turns it on? *Recommended: no;* the Vault covers the same work, and an agent-held value leaks into transcripts and other agents' reach (§9.2).
2. **Passwords and one-time codes.** The Vault stores logins (§5.2). Should agents fill them, with the same per-site approval? *Recommended: not yet;* sign-in is where phishing pages aim, and the person signing in is the proof they are there. A later spec can add it.
3. **Approval granularity.** First use per site (forms) and per agent (commands) asks, then "Always" (as §6.4 and §7.3 say)? Or every use asks? Or some entries never ask?
4. **Reveal protection.** OS confirmation (Windows Hello, Touch ID) for Reveal and Copy, or a Vault passphrase, or none?
5. **Existing secrets.** List provider account keys, MuxBus and browser logins read-only under "Kept by AgentMux" (§5.2), or move their management into the Vault?
6. **The `.vault` container.** age with its scrypt passphrase recipient (§8.2), or AgentMux's own header with Argon2id and XChaCha20-Poly1305? *Recommended: age;* no cryptography of our own, and the person can open the file without AgentMux. Argon2id resists GPU guessing better than scrypt, but a strong passphrase matters far more, and §8.2 suggests one.
7. **Plaintext export.** Offer CSV and JSON export behind a warning (§8.3), or only `.vault` and `.kdbx`? *Recommended: offer it;* people leaving for another manager need it, and most managers import only plaintext files.
8. **Bank numbers** (frames spec §8, question 1): account and routing numbers as secret fields, filled from the Vault? *Recommended: yes.*

## 14. Tests

- **Unit:** the `autocomplete` rule (§4.3); kind matching for every kind; taint survives a field turning non-secret; `Vault` `list` has no values, tails or grants; refusals name their reason; scrubbing replaces the plain, base64, URL-encoded and hex forms.
- **srv:** Vault writes refused on every agent route; `fill` with `vault` and `run` with `secrets` put no value in any response, error or log line (grep srv's and the host's logs in the test); approval answers refused without `X-Host-Token`; "Always" refused when the kind doesn't match; a revoked grant asks again.
- **Import and export:** a `.vault` round trip keeps every entry, kind, part and TOTP seed; a wrong passphrase and a tampered file are refused without partial writes; `age -d` opens an export and the JSON matches the documented schema; each importer against its fixture (real exports with test data: a KDBX 3.1 and a KDBX 4 file with Argon2 and a key file, a LastPass CSV with every `NoteType`, a `.1pux`, a password-protected Bitwarden JSON, …); duplicates detected; an oversize file, a zip bomb and a `.kdbx` with huge Argon2 parameters refused with their reason; no MuxAPI route can start an import or export.
- **Frontend:** the Vault pane hides values until Reveal; `UI` `screenshot`, `query` and `window` captures black it out; it relocks when idle.
- **End to end:** the triggering form (P1); an SSN fixture filled from the Vault, then screenshot, snapshot, `find` and read-back all masked; the same entry asked about on another origin; an injected page asking the agent to fill the SSN into a comment box gets the kind-mismatch question, and Deny leaves the box empty; a test command run with an API key from the Vault, its output scrubbed.

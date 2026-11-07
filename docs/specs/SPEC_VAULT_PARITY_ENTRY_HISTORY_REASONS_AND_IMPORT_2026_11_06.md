# SPEC: the Vault's feature set matches the secrets CLI: a reason and a history on every use and change, import from AWS Secrets Manager, verify and health

**Status:** active. Approved by the operator on 2026-11-06 (relayed by AgentY); §4's recommendations are the decisions. H1 is built (see "As built (H1)" at the end). Spec task #27968 (the card was filed as the import task and became spec-first on 2026-11-06).
**Author:** Agent2-6 (Vault). **Extends:** `SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md` (the Vault spec), `SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md` and `SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md` (§6.7 Activity, §11 AWS Secrets Manager).
**Verified on:** `main` at `1a79691b0` (2026-11-06).

## 1. Summary

The operator's words (2026-11-06, relayed by AgentY, quoted):
- "you add the key, take it from secrets and put it in the vault" (the first ask: an agent had no way to put a value it was given into the Vault; `Terminal` `run` with `capture` can do it on this machine, see §3, and the key is moved);
- "we essentially need the same feature set as what we get with secrets, any use of the vault requires a changelog (or a note regarding access)";
- "no i didnt mean exactly like our secrets tool works, we append the changelog directly into the secret bc we are forced to. inside agx we have flexibility, we can use a more robust model to keep this data. the point is leaving a note, how we encode it needs to be robust (not a text based changelog like the secrets tool's)".

So this spec does five things:
1. **A reason on every use and every change.** An agent says why when it uses an entry, changes one, or imports one. It is stored, shown in the approval card, and shown in the entry's history.
2. **A structured history per entry**, kept in srv's store next to the entry: one append-only row per write and per read or use, never holding a value, kept when the value rotates, tamper-evident, queryable by the pane, Audit and agents.
3. **`Vault import`**: srv reads a secret from AWS Secrets Manager itself, the person approves once with a card, the agent gets back only the entry id. It works for container agents through the broker.
4. **`show`, `update`, `history`, `verify` and `health`**, so an agent has what the `secrets` CLI gives (list, get, update, verify, health) without ever getting a value.
5. How each shows in the Vault pane and in Audit.

## 2. What exists today (from the code)

- `db_vault_activity` (`backend/storage/vault.rs`): one row per change or use: `at`, `entry_id`, `entry_label`, `actor`, `action`, `target`, `outcome`. Append-only in practice, kept after an entry is deleted. It has **no reason**, no version, no machine, no link to a task, PR or approval, and nothing makes a missing row detectable. The pane's Activity tab and the Audit view list it.
- `db_vault_versions`: an entry's earlier values, kept 7 days, with who replaced them and how.
- The agent tools that touch entries: `Vault` (`list`, `request`, `generate`, `rotate`), `Terminal` `run` (`secrets`, `stdin_secret`, `secret_files`, `capture`), `Browser` (`fill` with a Vault entry, `signin`, `capture`). `request` carries a free-text `why` that the card shows; none of the others carries a reason, and none records one.
- `Terminal` `run` with `capture` stores a command's output as a new entry and asks nobody for a new entry (generate spec §4.2).

## 3. What the secrets CLI gives, and the Vault's answer

| `secrets` | Vault | Notes |
|---|---|---|
| `list` | `Vault list` (exists) | Names, kinds and the Vault each is in, never values |
| `get` (the value) | **Never to an agent.** Replaced by *use*: `Terminal` `secrets`, `stdin_secret`, `secret_files`, `Browser` fill, `signin` | The person's own Reveal and Copy stay, behind the OS confirmation |
| `get --path` (one field's metadata) | `Vault show` (new) | Kind, label, plain fields, which parts are set, expiry, Vault, version count, last use. No tail, no length |
| `update` with changelog | `Vault update` (new, details only), `rotate` / `generate` / `import` with `id` / `capture` with `as: {id}` (values) | Every write needs a `reason` and is recorded (§5) |
| `import` (new) | `Vault import` (new) | §7 |
| `verify` (structure and changelog completeness) | `Vault verify` (new) | §8 |
| `health` | `Vault health` (new) | §8 |
| `escalate`, `auth-modes`, `npm-login` | none | Not Vault matters; escalation stays `secrets escalate` |

Native agents can already do the *first ask* today, without any new code: `Terminal` `run` with the command `secrets escalate admin --reason <reason> -- secrets get <secret> --path <key> --raw` and `capture` `[{as: {kind, label}, from: {stdout: all}}]` stores the value and the agent sees `[vault:<id>]`. **A job's command line is not parsed for quotes** (it is split at spaces and keeps the quote characters), so every argument must be one word (a hyphenated reason); this is what broke the first attempts. What capture cannot do: serve **container agents** (no host `Terminal`), ask the person first, or carry a reason. That is what §5 and §7 add.

## 4. Decisions to take (recommended answers first)

1. **Is `reason` required for an agent's use or change?** Yes, 8 to 300 characters, one line. A call without it is refused with a sentence saying what to add. The person's own actions need none.
2. **Does `Vault list` record?** No: it shows labels and kinds, no value, and an agent lists often. `show`, `history`, `verify` and `health` don't either. Everything that changes an entry, and every use that gives a value to a process, form or sign-in, does.
3. **Retention.** Keep every history row. At 100 rows a day a row of about 400 bytes is about 15 MB a year; revisit when the sync phases make rows travel. No compaction scheme: the operator asked for a robust record, not a text log to trim.
4. **May `import` read AWS with the person's host profile for a native agent?** Recommended yes, with the card naming the secret: the person's approval is what authorizes it, and an agent's own escalated session can't be handed to srv (an `eval`'d session lives in the agent's shell, not in srv). For a container agent, srv mints a session through the broker's grant for that agent (reason and hourly cap apply, §7).
5. **Tamper evidence.** A hash chain, with its head also kept in the Vault key's keychain item (§5.4). Cheap, and stated honestly as evidence, not proof (§9).

## 5. The history

### 5.1 One row per event

`db_vault_activity` grows (columns added, old rows keep their values; this is the same table the pane and Audit already read, not a second log):

| Column | Meaning |
|---|---|
| `seq` | Order on this machine (exists) |
| `at` | Time (exists) |
| `entry_id`, `entry_label` | The entry (exist); a deleted entry's rows stay, shown as "(deleted)" |
| `version` | The entry's value version this event made or used (a counter on the entry, new); 0 for details-only events |
| `actor_kind` | `agent`, `person` or `srv` (a sweep, an expiry reminder) |
| `actor` | The agent's id, or `you` (exists) |
| `host` | The machine's name (the instance key's name once sync exists); the agent's own host for a container agent |
| `action` | What was done, as the code already names it: `created`, `changed` (details only), `replaced` (a new value), `rotated`, `generated`, `captured`, `restored`, `moved to another Vault`, `deleted`, `imported`, `exported`; and for uses `filled a web form`, `signed in`, `ran a command`, `revealed`, `copied`, `sent on sign-up`, `filled on sign-up`. The list is not closed in code (a row is written for each of many events); the ones that give a value (`created`, `generated`, `captured`, `replaced`, `rotated`, `restored`) are the versions, and the ones that hand a value out are the uses |
| `target` | Where it went (the site, the command, the Vault move) (exists) |
| `outcome` | `done`, `denied`, `failed: <why>`, `declined by silence` (exists) |
| `reason` | The agent's reason, clipped to 300 characters. Empty for the person's own actions and for rows made before this change (shown as "(before reasons were recorded)") |
| `refs` | Up to 3 short references the event relates to, given by the agent or made by srv: a task (`#27968`), a PR URL, the approval's use-request id. Not parsed; shown as given |
| `prev_hash`, `hash` | §5.4 |

**Never in a row:** a value, a tail, a length, a hash of a value, a part of one. The history of a secret must not help guess it. A command's text is clipped and cleaned of values as it is today (the scrubber).

### 5.2 Kept when the value rotates

A rotation, a restore or an import with `id` adds rows and a new `version`. Rows keep the `version` they happened under, so "who used version 4 of this key" is a query, also after version 4's value is gone (7 days). Moving an entry between Vaults adds one `moved` row and changes none.

### 5.3 Who may read it

- The person: all of it, in the pane and in Audit.
- An agent: `Vault history <id>` returns the rows of an entry **in a Vault that agent may use** (the VS1 rule: an entry it may not use is refused like an unknown id). It sees actors, actions, reasons and refs, never a value. This is how an agent answers "why was this rotated?" before rotating again.
- A synced machine (VS2): each row is a record of the layer, as §6.7 of the shared spec already says; `host` becomes the machine's name.

### 5.4 Tamper evidence

`hash = SHA-256(prev_hash || seq || at || entry_id || version || actor_kind || actor || host || action || target || outcome || reason || refs)`, each field length-prefixed so two rows can't be re-cut into each other. The chain runs across the whole table in `seq` order. After each append, the head (`seq` and `hash`) is written into the machine-sealed record that holds the Vault key's metadata, so removing or editing rows in the database alone breaks the chain at that point **or** disagrees with the head. `Vault verify` (§8) walks it and names the first broken `seq`. A row can't be inserted in the middle without recomputing everything after it, and recomputing needs the head to be rewritten too, which takes the keychain.

This is evidence for the person, not proof against someone who holds both the database and the keychain, and the spec says so in the pane's help.

### 5.5 How an agent gives the reason

A `reason` string on:
- `Vault` `request` (the existing `why` stays as an alias), `generate`, `rotate`, `update`, `import`;
- `Terminal` `run` when it uses `secrets`, `stdin_secret`, `secret_files` or `capture`;
- `Browser` `fill` with a Vault entry, `signin` and `capture`.

The reason is the agent's own claim. It is shown quoted, as "<agent> says: <reason>", and is never used to decide anything (an approval is the person's). The tools' descriptions say so, and say what makes a good one ("what and why, for which task"). A missing or too-short reason is refused before anything is asked or written. `refs` is optional text next to it (`task`, `pr`).

In the approval card, the reason is a line under the title (`UseRequest` gains `agent_reason`, clipped for the card). The Audit row of a use carries the same reason in its existing `reason` field.

## 6. Where it shows

- **Vault pane, entry row:** a History icon opens a dialog: a table of When, Who (with the machine when there are several), What, Where, Version, Why, Outcome, newest first, filters "Changes" and "Uses". It reads `history`. A row before reasons shows the "(before reasons were recorded)" text.
- **Vault pane, Activity tab:** gains a Why column and links the entry to its History.
- **Audit:** each use through AgentMux already writes an Audit row; it gains the reason, and a link that opens that entry's history at that row. A history row made by srv itself (an expiry reminder) shows as `srv`.
- **Agent pane:** the card's line, §5.5.
- **Look checks:** macOS and Linux, dark and light, for the History dialog, the Activity column and the card's new line (the workspace rule for visible changes).

## 7. `Vault import`

`Vault` action `import`, arguments:

```
source:  { aws_secret_id, key?, region?, profile? }   # key: a field of a JSON secret, dotted path
kind, label, fields?, vault?                           # what the entry is, as for `generate`
id?                                                    # replace this entry's value instead of making a new one
reason, refs?
```

1. **Before anything is asked:** the arguments are checked (secret id and key use a narrow character set; region matches an AWS region; `kind` has a main part; `vault` is one the agent may use, else the agent's first), and `id`, if given, must be an entry in a Vault the agent may use (else "no such entry").
2. **The card.** One approval, every window, in the asking agent's pane like the other cards: "<agent> wants to copy a secret from AWS Secrets Manager into your Vault as <label>", with the secret id, the key, the region, the Vault it goes into and the agent's reason; "The agent never sees the value: AgentMux reads it." **Allow once** or **Deny**, no Always (a read of a new secret is a decision each time), and silence **denies** (the card never goes ahead on silence). An import with `id` says "replaces the value of <label>; the old one is kept 7 days".
3. **The read, after approval, by srv.** AWS is read with the host's AWS CLI as the broker already does (`HostRunner`, a clean environment, no window):
   - **Container agent:** srv asks the broker for a session for the grant named in `profile` (the grant's open-to list, `needs_reason`, hourly cap and Audit row apply unchanged, with the import's reason as the session reason), then runs the AWS CLI's Secrets Manager read for that secret id (with the region, if given, and JSON output) with those credentials.
   - **Native agent:** the same command under the host's own AWS setup, using `--profile <profile>` if given. The person's approval of this card is the authorization (§4.4).
4. **Storing.** srv takes `SecretString`, picks `key` if given (a string or number; anything else is refused), stores the value as the entry's main part in the chosen Vault, records `imported` with actor the agent, target `aws-sm:<id>[#<key>]`, the reason and refs, and answers `{id, label, vault}`. Binary secrets are refused with a sentence.
5. **Nothing leaks.** The value lives in `Zeroizing` buffers from the CLI's stdout to the sealed row; the CLI's stdout is never logged, put in an error, or kept; only its stderr's first lines (AWS's own error text, which holds no value) reach an error message. The response, the log lines, the Audit row and the history row never hold it.
6. **Refusals say why:** a denied AWS read ("AWS refused: this identity can't read that secret"), a lapsed host login (the broker's existing sentence), a missing CLI, a grant that isn't open to this agent, no entry stored. A declined or unanswered card stores nothing and records the `denied` outcome.
7. **Container agents** reach the route like every Vault route (signed identity); nothing about it needs the host's shell.

`Vault import` copies once. The continuing link (an entry that reads Secrets Manager at each use) is **Backed by** in VS4 of the shared spec §11.1; this shares its resolver.

The secret path is a private detail: it lives only in the person's own history, never in a public repo; tests use invented ids.

## 8. `show`, `update`, `history`, `verify`, `health`

- **`Vault show <id>`:** the entry's kind, label, plain fields, tags, which parts are set, expiry state, Vault, version count and last use. For an entry the agent may use.
- **`Vault update <id>`:** label, plain fields and tags only. A value never changes through `update` (that is `rotate`, `generate`, `import` with `id` or `capture`). `reason` required; each change is a `changed details` row with what changed in the clear fields (field names only, not their values, if the field is one a Vault hides such as a card's expiry; plain fields as today are not secret).
- **`Vault history <id>`:** §5.3.
- **`Vault verify [<id>]`:** per entry: the kind is known; every required part is set; the plain fields satisfy the kind; the expiry is parseable; the history rows for it are present from `created` on and their chain verifies; the sealed row opens with the machine key (a boolean, never the value); a backed entry's reference parses (VS4). Without an id, all entries the agent may use plus the table's whole chain. Answers a list of `{id, problem}`; a problem line never quotes a value.
- **`Vault health`:** the key store's state (readable, which backend), counts (entries, Vaults, expiring within 14 days, expired, unused for 90 days), the history chain's state, pending requests and uses, the version of the Vault schema. No value.

`verify` and `health` need no reason (they read no value and change nothing).

## 9. Security and trust

- **The reason is untrusted text.** Clipped, one line, shown quoted and attributed ("Agent X says"), no markup, never decides an approval. A card never shows a reason as the operator's words.
- **No value leaves srv** by any of these routes. The new routes return ids, names, states and rows of history.
- **Host-only things stay host-only:** moving an entry, deleting a Vault, choosing a sign-in Vault, reveal and copy.
- **Agents see only the Vaults they may use**, for `show`, `history`, `update`, `verify`, `health` (counts only over the Vaults it may use) and `import` targets.
- **Import widens nothing on its own:** it reads a secret the person approved by name; a native agent can't read AWS more widely through it than through the person's host profile, and the card names the one secret.
- **The hash chain** detects an edit or a deletion of history rows made without the keychain. It does not stop one made with both; the pane's help says so.
- **Public repo hygiene:** this document, its tests and its fixtures use invented secret ids and no account numbers.

## 10. Phases (one PR each, in this order)

| PR | What | Depends on |
|---|---|---|
| **H1** | History columns, `version`, the hash chain and its head, `record` taking reason, refs, actor_kind and host; `Vault history`; the pane's History dialog and the Activity column | Nothing |
| **H2** | `reason` required on `Vault` `request`/`generate`/`rotate`, `Terminal` `run` with secrets or capture, `Browser` `fill`/`signin`/`capture` (MCP schemas and descriptions, srv refusals); the card's line; Audit's reason and link | H1 |
| **H3** | `Vault import` (new and with `id`), the card, the broker path for containers, tests | H1, H2 |
| **H4** | `Vault show`, `update`, `verify`, `health` | H1 |

## 11. Tests

- **History:** a row for every action in the closed list, none with a value, tail, length or hash of one (a test that scans every row's text for the values used in the suite); versions survive a rotation; a deleted entry's rows stay; the chain verifies, and changing, deleting or inserting a row, or changing the head, is found at the right `seq`; an agent reads history only for Vaults it may use.
- **Reasons:** each tool refuses a call with no or a short reason, before anything is asked or written; the reason reaches the card, the history row and the Audit row, clipped; an `ask`ed use shows "says".
- **Import:** the value is in no response, log line, error, Audit row or history row (the scan); a denied AWS read gives the sentence and stores nothing; a declined or unanswered card stores nothing; an import with `id` keeps the old value as a version; a container agent's session is minted under its grant, with the grant's checks; a grant not open to the agent is refused; a bad secret id or key is refused before the card; silence denies.
- **`verify` and `health`:** a broken chain, an unset required part, an expired entry and an unreadable key store each show up; none quotes a value.
- **Pane:** History dialog (rows, filters, the pre-change text), the Activity column, the card's line; macOS and Linux look checks.

## 12. What this spec does not do

- Sync of history between machines (VS2/VS3 carry the rows as records of the layer, §6.7 of the shared spec).
- Backed-by and Mirrored entries (VS4).
- Reasons for the person's own actions.
- Verifying that a reason is true. It is the agent's statement, kept so the person can read it later.

## As built (H1)

- **Columns** on `db_vault_activity` (added with `VAULT_ALTERS`; rows made before keep empty values and have no hash): `version`, `actor_kind`, `host`, `reason`, `refs` (a JSON array of up to 3 short strings), `prev_hash`, `hash`.
- **Version** is derived by the store in the same transaction as the insert: the count of the entry's value-giving rows that went through since it was last deleted, one more for a row that is one; a use is under the version then current; any other row is 0. A deleted entry's id coming round again starts at 1 and is not lent the first entry's rows.
- **Chain:** `hash = SHA-256` over the previous row's hash and the row's `seq`, `at`, `version`, `entry_id`, `entry_label`, `actor_kind`, `actor`, `host`, `action`, `target`, `outcome`, `reason` and `refs`, each length-prefixed. The newest row's `seq:hash` is also kept in the keychain item `vault-history-head`, written at once or at most every 30 s (a write follows a burst, so it catches up). `history::verify_chain` names the first row that is wrong: a changed row, a removed row (the next one no longer follows), a head whose row is missing or different (the newest rows were removed). Rows made before the chain, and rows with no hash written after it began by an older AgentMux sharing the store (a mixed-version computer), are counted and not checked, and the dialog says how many: such a row has none of the chain's columns (version, actor kind, machine, reason, references, previous hash). A row with no hash that still has some of them was taken out of the chain, and is reported as a change at that row. A row blanked of all of them by hand in the middle of the chain is found at the next row, which was chained to it; one blanked at the very end looks like an older build's, which is why such rows are counted and shown rather than hidden, and the keychain's copy of the newest line is what catches the rest. When the keychain can't be read or holds no head yet, the check says so (`head_checked` false) and the dialog notes that lines removed from the end would not show.
- **Reading it:** the pane's host op `history` (an entry's rows) and `history_check` (the chain's state, shown under the table); the agent route `POST /api/v1/vault/history` and `Vault` `history` (id, limit), open to container agents like the other Vault routes, for entries in Vaults the proven agent may use (anything else answers "no entry <id>", as for an id that isn't there).
- **Reasons** are stored by `record_row` (`VaultActivityRow.reason`, `refs`) and cleaned to one line of at most 300 characters; no tool supplies one until H2, so every row an agent makes now shows "(no reason given)" and rows made before this change show "(before reasons were recorded)".
- **Pane:** a History icon on each entry row opens the dialog (When, Who, What, Where, Version, Why, Outcome; Changes and Uses filters); the Activity tab has a Why column and its entry names open the same dialog.

## As built (H2)

- **One rule** (`history::require_reason`): the reason is one line of 8 to 300 characters; a missing, blank, short or long one is refused with a sentence saying what to add, before anything is asked, run or written.
- **Where it is required:** `Vault` `request`, `generate` and `rotate` (`why` still works as the older name on `request` and `generate`); `Terminal` `run` when it uses `secrets`, `stdin_secret`, `secret_files` or `capture`; `Browser` `fill` with a Vault entry (not `request_value`, where the person types the value), `signin` and `capture` (the `reason` argument `Browser` already had for handoffs). A rotation's own commands carry the rotation's reason.
- **Where it shows:** the approval card has a line "<agent> says: "<reason>"" (`UseRequest.agent_reason`, quoted, clipped to 140 characters on the card (the whole in a tooltip), never the person's words, deciding nothing), and every history row an agent's use or change writes carries the reason (the `Line` and `record_line` helpers).
- **Audit:** the Audit view indexes the agent's own tool call, which now carries the reason as an argument, so no separate Audit field or link was built; the entry's History is where the reason is read with the rest of the entry's record.
- **Not covered:** a `Terminal` job that is not given Vault entries needs none, and neither does `Browser` `request_value`.

## As built (H3)

- **Route and tool:** `POST /api/v1/vault/import` and `Vault` `import` (`source` { `aws_secret_id`, `key`, `region`, `profile` }, `kind`, `label`, `fields`, `vault`, `id`, `reason`, `refs`, `wait_secs`), open to container agents like the other Vault routes. The answer is `{status, id, label, vault}`; nothing else comes back.
- **Checked before the card** (`aws_import::check`): the reason (the H2 rule), the secret id (letters, digits and `/_+=.@:-`, at most 512, never a leading `-`), the key (a dotted path of letters, digits, `_` and `-`), the region (an AWS region's shape), the profile, the kind, the label and fields, the Vault (one the agent may use, else its first) and, with `id`, that the entry is in a Vault the agent may use and is of the kind named. For a container agent, the grant in `profile` must exist, be on and be open to it (`check_grant`); the broker checks again when it mints.
- **The card:** `UseRequest.import` (`UseImport`: service, secret, key, region, the AWS profile or grant the read runs under, Vault, whether it replaces) with the agent's reason as the "says" line. The secret leads the card, a long id is cut in the middle (its name is at the end; the whole in the tooltip), and the region stays whole. Silence ends as `expired` and the agent is told a copy of a secret needs a yes. It offers Allow once or Deny, never Always, and `uses::goes_ahead` is false for it whatever the kind, so silence denies (`Ended::Expired`, recorded as `expired`).
- **The read:** `aws_import::read_secret` runs the host's AWS CLI through the broker's `HostRunner` (clean environment, no window, 25 s): a Secrets Manager read of `SecretString` as JSON, with `--region` if given. A container agent gets a session from `credentials_for` for the grant in `profile` (its checks, hourly cap and Audit row apply, with the import's reason as the session reason) and the CLI is given those credentials; a native agent runs under the host's own setup (`--profile` if named).
- **What is kept:** the whole text, or the string or number at `key` in a JSON secret (anything else, a secret that isn't JSON, a missing key, an empty value, a binary secret or one over 1 MiB is refused with a sentence). A new entry takes it as its main part in the chosen Vault; with `id` it replaces the main part and the old entry is kept 7 days as a version marked `imported`, in one transaction.
- **The record:** an `imported` line (a value-setting action, so it counts toward the entry's version and resets an expiry reminder) with the agent, target `aws-sm:<id>[#<key>]`, reason and references. A denied, unanswered or failed import records the same line with that outcome and stores nothing.
- **Nothing leaks:** the CLI's stdout is wrapped in a zeroizing buffer at once and is never logged, put in an error or kept; what is parsed from a JSON secret is wiped before it drops. Errors are fixed sentences (AWS refused to read it, no such secret, the host's login has lapsed, no CLI, took too long) and, for an unknown failure, one cleaned line of the CLI's stderr with account-number-length digit runs cut out. The tests scan the answer, every error and every history row for the value.
- **Also in this change:** the two bookkeeping rows H2 left without a reason (`requested an entry`, and `asked to use it` after a silent go-ahead) now carry it.

# Spec: The Vault as the central index of keys: linked entries, a Fleet Vault, and bulk import with one approval per batch

**Status:** active. The spec task is complete and the build is under way: B1 is built (#7487, with follow-ups in #7492), B3 is built (#7498, see "As built (B3)" at the end) and B2 is built (#7501, see "As built (B2)"), B4 is built (#7506, see "As built (B4)") and B5 is built by the PR that adds "As built (B5)".
**Task:** #47647 (the build is #47648). **Author:** Agent2-6 (Vault). **Date:** 2026-11-15.
**Asked by the operator (2026-11-14):** "lets audit all our keys and make sure they all link centeraly to the vault"; and, on the recommendation to build a bulk approval first and a second Vault: "great, go with recommendations".
**Public repository.** This spec and its tests use invented names (`example/app`, `EXAMPLE_TOKEN`) and no counts of a real inventory. The inventory itself stays in a private report.

## 1. What this adds, in one page

Today the Vault holds values. A key that a machine role reads (a cloud function reading a secret store, a workflow reading a CI secret) must stay where that role can reach it, so it never gets into the Vault, and the Vault cannot tell the person it exists, who reads it, when it expires or whether it changed. Three small additions fix that without moving a value anywhere it must not be:

| Part | What it is | Builds on |
|---|---|---|
| **A linked entry** (§2) | An entry that says where a key lives and who reads it, and holds no value | The source link (#47649): `source_*` fields and the `check` |
| **The Fleet Vault** (§3) | A second Vault, named Fleet, open only to named agents, for infrastructure keys and the links to them | Several Vaults (VS1, built) |
| **A batch import** (§4) | One approval for a named list of keys, instead of one per key | `Vault import` (one secret per approval, built) |

The import plan (§5) puts them in the order that asks the person for the fewest approvals. Rollout across machines is §6.

Not in this spec: moving or narrowing who can read a secret in the cloud (the access spec, its own task); sealing signing keys held by AgentMux itself; sharing a Vault across machines (the shared-Vault spec's VS2 and VS3).

## 2. A linked entry

### 2.1 What it is

A **linked entry** is a normal Vault entry that **has no value** and says, in plain fields, where the key lives and who reads it. It has a label, a kind (almost always `api-key`), a Vault and grants like any other entry.

- It is not a new kind. Every kind already allows the `source_*` fields (§11.3 of the shared-Vault spec, #47649), and a read path already answers "no value saved" for an entry whose main part is empty. A new kind with no parts would break every place that reads a kind's first part.
- Marked by one plain field, `link`, set to `only`. An entry with a value and a source link (a "machine copy" of a value the Vault holds) is not a linked entry; it stays as built in #47649.
- Its fields, all plain and none a value:

| Field | Meaning |
|---|---|
| `source_kind`, `source_location`, `source_key`, `source_region`, `source_profile` | Where the key lives: as #47649. `source_kind` is `aws-secret`, `github-secret` or `other` |
| `readers` | Who or what reads it, one line each, up to 8: `<name> (<lambda, workflow, cli, person, agent or other>): <how>`, for example `invoice-sync (lambda): at run time`. Free text the person or the importing agent writes; the Vault does not enforce it |
| `expires` | The existing expiry field, when the key has one on record |
| `rotated` | When it was last changed, as recorded (a date) |
| `copies` | Other places the same key is kept, up to 4, each `kind: location#key` |

### 2.2 What a person sees

The entry shows in the Vault pane like the others, with a **Linked** badge and a second badge, **Readable outside the Vault**, because a machine role (and anyone with that role's access) reads the key without asking the Vault. There is no Reveal, Copy or Edit of a value, because there is none. The entry shows the place, the readers, the expiry, the date it was last checked and the result. "Open place" is a copy of the location, not a link into a console.

### 2.3 What an agent sees and can do

- `Vault list` and `show` give the label, kind, location, readers, expiry, rotated, copies and the last check's result. Never `source_profile` (as #47649 decides).
- `Vault use` (a request, a fill, a command's `secrets`) of a linked entry is refused with one sentence: "This entry holds no value: the key is kept at `<kind>: <location>`. Ask the person, or use the access you already have there." Nothing is asked of the person, because there is nothing to reveal.
- An agent may make or change a linked entry with a reason. Because the place and the readers become what the Vault tells the next agent, an agent's change to them is a card for the person, as for #47649's source (silence is a no). The importing batch (§4) is one such card.

### 2.4 How it is kept true: the metadata check

`Vault check <id>` on a linked entry reads **only metadata** of the place, never the value:

- the place exists and the asker's access can describe it (a secret store's description call; a CI secret's listing entry);
- its **last changed** date and, where the store has it, its rotation setting;
- for a `copies` place, the same.

It answers `present`, `changed since recorded` (the place's last changed date is after the entry's `rotated`), `missing` or `could not read`, with why. It never reads, compares or hashes a value, so nothing needs approval, and no value reaches srv. Each check is a line in the entry's history naming the place, as #47649's. The pane shows "last checked" and marks an entry **unchecked for N days** after 30. An entry whose `expires` is within 14 days, or whose check says `changed since recorded` or `missing`, shows in the Vault's health counts and in the existing expiry reminders.

A linked entry cannot tell whether two copies hold the same value (that needs a read). That is a later step (§8, open question 4).

### 2.5 What the check does not do

It does not rotate, set or push anything. Writing a value to a place is "Mirrored to" (the shared-Vault spec's VS4) and costs money in a cloud account; it stays a separate, later step.

## 3. The Fleet Vault

### 3.1 What it is

**Fleet** is a Vault like any other (VS1, built): a name, an `agents` setting (all, none, or named by id), entries, grants. It exists so infrastructure keys, host and machine passwords and the links of §2 do not sit beside the person's own cards, bank numbers and sign-ins in **Personal**, and so the people who can use them are a short named list instead of every agent.

- **The person makes it** in the Vaults dialog, and chooses its agents. Nothing creates it automatically, so no agent decides who is in it. A batch that names a Vault that does not exist is refused with that sentence.
- **Its agents setting is `named`** and starts empty: an empty list is "none", so a new Fleet is usable by no agent until the person adds some.
- **It is not a sign-in Vault.** A browser profile never fills a sign-in from it.
- **Its entries are ordinary entries:** a host password as a `login` (the host in `site`), a key as an `api-key`, a link as an `api-key` marked `link: only`. Move to, grants, history, versions and the approval card are unchanged.
- **Linked entries default to Fleet** in the batch import, because they describe infrastructure. The person can choose another Vault in the card.

### 3.2 How it relates to the existing settings

- The per-Vault `agents` setting is the only access rule added. There is no Fleet-only code path.
- Container agents follow the same rule through their host's srv (the shared-Vault spec, §10).
- An older AgentMux on the same computer ignores the agents setting (VS1a's note): the Fleet's limit is enforced from the build that carries VS1 on.
- Fleet is not a security boundary against an agent that can read the place itself (§2.2 says so on the entry). It is a boundary for what the Vault shows and answers.

## 4. Bulk import with one approval per batch

### 4.1 The action

`Vault` action `import_batch`:

```
batch:   a short name, up to 60 characters ("host passwords", "app signing material")
vault:   a Vault the agent may use; one for the whole batch
reason:  why, one line, as `import`
source:  { profile?, region? }          # one account and region for the whole batch
items:   [ { aws_secret_id, key?, kind, label, fields?, id?, link? }, ... ]   # up to 40
```

- `link: true` on an item makes a **linked entry** (§2) and reads no value; without it the item is an import exactly as `Vault import`, and `id` replaces an existing entry's value (the old one kept 7 days).
- A batch mixing profiles or regions is refused ("one batch, one account and region"); so is one that names two Vaults, a duplicate label or secret id, an entry the agent may not use, or more than 40 items. Every item is checked as `import` checks one **before anything is asked**.
- Linked and value imports may share a batch.

### 4.2 The card

**One approval, every window**, in the asking agent's pane, as `import`'s. It is a real approval of a real list:

- Title: "<agent> wants to copy N secrets from AWS Secrets Manager into your Vault: <batch>", or "to record where N keys live", when every item is a link.
- The account (profile and region) and the Vault, once, at the top; the agent's reason, clipped, quoted and attributed ("Agent X says").
- **A row for every item**: the label, the secret id and key (clipped, whole on hover), and "new", "replaces <label> (old kept 7 days)" or "link only (no value read)". A batch longer than the card's height scrolls; none is summarised.
- **A checkbox per row, all ticked.** The person may untick rows; Allow applies to the ticked ones and is off when none is. Unticked rows are recorded as not approved.
- "The agent never sees a value: AgentMux reads them." Buttons **Allow these N** and **Deny**. **No Always.** Silence denies, for the same window as a single import.
- The card names who may use the Vault, so the person sees that "Fleet" is open to named agents.

### 4.3 What runs after Allow

1. srv takes the **list as approved** (the ticked items, their order and every field above) and fingerprints it. It runs exactly that list; any item not on it is refused. An agent cannot add an item after the approval.
2. **One AWS session per batch.** For a container agent, srv asks the broker for one session for the named grant with the batch's reason (the grant's open-to list, `needs_reason`, hourly cap and Audit row apply unchanged; the cap counts one). A native agent uses the host's own setup under the approval.
3. Each value item is read and stored as `import` stores one: in `Zeroizing` buffers from the CLI's output to the sealed row, never logged, never in an error, never in the answer. Each link item makes its entry and reads nothing.
4. **No rollback.** Each entry is its own sealed row with its own history line, and a replaced value is kept 7 days. A failed item (refused read, binary secret, missing key) is recorded and the rest continue. The answer is one row per item: `{label, id}` or `{label, error}`, with the same sentences as `import`.
5. **History:** each item gets its own `imported` line (actor the agent, target `aws-sm:<id>#<key>`, the reason), and each carries `batch:<batch id>` in its refs, so the pane's Activity can show a batch together. One more line records the batch: the count approved, the count stored, the count failed, and the unticked ones.

### 4.4 Security notes

- The approval covers what it shows: the list, one account, one Vault. A batch can never read a secret the person did not see by name.
- A native agent cannot read more through a batch than through the host's profile: the approval authorises the named secrets only (as `import`, §4.4 of the parity spec).
- The reason is untrusted text: one line, clipped, quoted, never an instruction.
- No new human-only gate other than this approval: `import` already needs one per key, and this replaces N approvals with one that shows N rows.
- The batch is the person's decision each time. There is no "approve batches from this agent".
- Public-repo hygiene: tests and fixtures use invented ids; the card's text in the repo has no real names.

## 5. The import plan: batches, and what each needs

Each batch is one approval. In the order that asks the person least, with build numbers from §7.

| # | Batch | What | Needs | Held until |
|---|---|---|---|---|
| 1 | **Links for what the Vault already holds** | Add the source link to each entry whose value also lives elsewhere (#47649's fields), by the person in the pane or by one batch of `update`s. No value moves | B1 (#47649) | Nothing |
| 2 | **Machine and host passwords** | The passwords people and agents use by hand, as `login` entries in Fleet (host in `site`) | B3 | The deletion of the duplicates in the old place waits for the access spec |
| 3 | **Signing material for apps** | Each key and its password and certificate, with a `copies` line for the CI secret that carries the machine copy | B3 | The old place's removal waits for the access spec |
| 4 | **Catalog and provider keys** | Keys a person or agent uses by hand: signing seeds, AI-provider and data-provider keys, whichever are still used | B3 | Nothing |
| 5 | **Linked entries for keys machine roles read**, one batch per reader | So each batch can be checked against that one reader's code (one service's keys at a time). Link-only, no value | B2, B3 | Nothing |
| 6 | **Settings keys** | Nothing to import where a setting holds no plain key; on a machine with one, "Move to the Vault" in Settings does it | Built (#47636, #47654) | Nothing |

Notes:
- **Deletion is not an import.** Anything that goes from a cloud store or a CI repo is a list the owner of each item confirms and the person approves separately; the Vault never deletes at a place.
- **What the access spec changes.** Narrowing who can read a cloud secret (moving widely readable secrets under a role-only prefix) is its own task. It does not block a link, because a link only records where a key is; it does block removing copies after an import, and any rotation of a key that was readable too widely.
- The person is asked about six times in all, not once per key.

## 6. Rollout across machines

The Vault is per machine today (the shared-Vault spec's VS2 and VS3 are not built). Entries made on one machine exist on no other.

- **R0, now:** the batches run on one machine, with the person present. Links describe cloud places that look the same from every machine; they are just not visible elsewhere.
- **R1, stopgap, no new code beyond B5:** Fleet's **links** export as a `.vault` with values left out (a linked entry has none to leave out; a value entry is skipped by the "links only" option), and each other machine's steward imports it. The person chooses it; the export is encrypted with a passphrase as `.vault` is today.
- **R2, with VS2:** Fleet is shared like any Vault. Links travel first, because they hold no value; entries with values join on the person's choice.
- **Per host, the keys that never go into the Vault:** a machine's own role credentials (the key that proves a host to the cloud) are not entries; they are what a check uses. The spec does not move them.
- The machine-by-machine check of local items (OS keychain items, plain settings keys, `.npmrc` tokens) is the steward's list in the private report, not a Vault feature.

## 7. Builds, in order

| Build | What | Size | Needs |
|---|---|---|---|
| **B1** | The source link and its `check` (#47649, built) | small | Nothing |
| **B3** | `Vault import_batch` and its card; `batch:` refs and the Activity grouping | small to medium | B1 not required; reuses `import` |
| **B2** | Linked entries: the `link: only` field, `readers`, `rotated`, `copies`; the metadata check; the badges and "unchecked" state; the refusal sentence on use | medium | B1 |
| **B4** | Fleet in the pane: batch cards that name a Vault open to named agents; the "Readable outside the Vault" badge; a Fleet filter | small | B2 |
| **B5** | The "links only" `.vault` export (R1) | small | B2 |

B3 goes before the imports, because without it group 2 to 4 are many approvals each. B2 is needed for group 5 only.

A look check on macOS and Linux (dark and light, narrow and wide) before the cards of B3 and B4 merge, as for any visual change.

## 8. Decisions for the operator

1. **A linked entry is an ordinary entry marked `link: only`** with no value, not a new kind. Recommended: yes (§2.1; a kind with no parts would break the readers of a kind's first part).
2. **Fleet is made by the person, starts open to no agent, and linked entries default to it.** Recommended: yes (§3.1).
3. **A batch card shows every row, lets the person untick rows, has no Always, and silence denies.** Recommended: yes (§4.2). The cap is 40 items per batch.
4. **No value comparison for linked entries in this build.** Whether two copies hold the same value needs a read; the existing `check` of #47649 does it for an entry that holds a value. Recommended: leave it out of v1.
5. **Rollout stopgap R1 (links-only export to the other machines).** Recommended: build B5 now, because it costs little and shows the links on every host before VS2.

## 9. Open, and not determined

- Whether the broker's one session per batch is enough for a batch longer than a session's lifetime. It should be (a read is seconds); the first run tells.
- How the "unchecked for 30 days" state should be run: on opening the pane, or on a schedule. The existing expiry reminders run on a schedule; reusing them is the likeliest.
- Whether `readers` should offer a pick-list (the people's and agents' names the Vault already knows) beside free text.
- The access spec's list of secrets to narrow is its own work; this spec records only that deletion of copies waits for it.

## As built (B3)

Agent2, 2026-11-15, task #47648. `Vault` `import_batch`, its card, the `batch:` references and the Activity grouping, as §4 says, with these choices made while building:

- **A duplicate is the same secret and the same key.** Two fields of one JSON secret (`hosts/alpha` with key `username`, and with key `password`) are two rows. The same secret with the same key twice is refused, and so are two rows for one entry (two labels that differ only in case or spaces, or one `id` twice).
- **One account and region, checked where it can be.** An item that names a region, a profile or a Vault other than the batch's is refused. Secrets named by ARN must all be in one account and one region, and in the batch's region when it names one. A secret named by its name is in whatever account the profile reads.
- **`link: true` was refused until B2**, which makes the linked entry and gives a batch of links only its own title (see "As built (B2)").
- **Who may use the Vault is not on the card yet.** That line is B4's, with the Fleet Vault.
- **The rows the person left out are named in the batch's own line** ("not approved: ..."), and have no line of their own. A batch that was denied, or that nobody answered, is one line.
- **The batch's own line** has the action `imported a batch`, the list's name where an entry's label goes, and "N of M approved, S stored, F failed; list <fingerprint>", with the outcome `done`, `partly done` or `failed`. The fingerprint is of the approved rows; the answer to the agent carries it as `list`.
- **A batch's reads have five minutes together.** A row not reached by then answers "not reached" and is recorded as failed; the rows already stored stay.
- **The answer** is `{status, batch, name, vault, list, stored, failed, items: [{label, id} | {label, error}], not_approved: [label]}`.
- **In the Activity tab** a batch's own line has a button with the count of its secrets, which shows their lines under it. The lines of a batch whose own line is older than the list goes back stay where they are.

## As built (B2)

Agent2, 2026-11-15, task #47648. Linked entries, as §2 says, with these choices made while building:

- **The mark is set when the entry is made.** `link: only` goes on a new entry: a `link: true` item of `import_batch`, or the "Link only" switch of the pane's form for a new API key, login or note. No `update`, by an agent or the person, makes an entry that holds a value a link, or gives a link a value: the answer says to make a new entry. An `import` with a link's `id` is refused the same way.
- **A link needs a place** (`source_kind` and `source_location`) and holds no part at all. `readers` (up to 8 lines of 200 characters) and `copies` (up to 4 lines, each `kind: location#key`, each held to what that kind of place may be) may be said of any entry, so an entry that holds a value can say where else the key is kept. `rotated` is a link's own: the Vault's history dates a value it holds.
- **A link item of a batch** is an API key when it names no kind, takes its place from the item's secret and key and the batch's region and profile, and reads nothing: a batch of links only asks the broker for no session. Its line in the history has the action `linked` and the place as its target, with the batch's reference. The card's row says "link only (no value read)", and a batch of links only is titled "wants to record where N keys live".
- **The use of a link** (a fill, a command's `secrets`, a version of it) is refused before anyone is asked, with §2.3's sentence naming the place as `kind: location#key`.
- **An agent's change** to `readers`, `copies` or `rotated` is a card for the person, like a change of place, on any entry. The card shows what is recorded now and what is asked for. A change to other fields (notes, a label) asks nobody. The Vault tool waits for the person's answer on such an update, as on a change of place (the live run found it giving up after ten seconds with the card still open).
- **The check describes an AWS secret only.** For a place in AWS it reads the secret's description (when it last changed, whether it rotates, whether it is being deleted) and never the value. It answers `present`; `changed since recorded` when the place's day of last change (UTC) is after the entry's `rotated`; `missing` when there is no such secret or it is scheduled for deletion; `could not read` with why. A link with no `rotated` is `present`, and the answer carries `last_changed` and `rotation` as the place gives them. A CI secret or any other place answers `could not read`: describing a CI secret needs a GitHub login chosen for it, which is not built.
- **Copies.** A copy in AWS is described under the entry's own region and login, and only when the entry's own place is in AWS (there is no other login to describe it under). Each one asked has its own history line, `checked a copy`, so the newest `checked` line is always the entry's own place and a check counts once toward the agents' six an hour. Every copy is in the answer.
- **Unchecked.** A link whose newest check is more than 30 days old says "unchecked for N days" on its line; one never checked says "not checked yet", as a machine copy does. The check runs when asked, by the person's button or an agent's `check`; nothing runs it on a schedule (§9 leaves that open).
- **Health** counts `linked`, `linked_changed_or_missing` and `linked_unchecked_for_30_days`, and no longer counts a link as unused. `verify` finds a link with a value wrong, and not one without. An expiry reminder follows a link's `expires` as any entry's. A check that says changed or missing has no reminder of its own yet.
- **In the pane** a link has the badges "Linked" and "Readable outside the Vault", "no value kept" where the last characters go, "Kept at:" with the newest check, then who reads it, the date recorded and the other places. It has no Show or copy; "Copy the place" copies the location. The form shows no value input for one.
- **`list` and `show`** give `link_only`, the place, `readers`, `rotated`, `copies` and the newest check; never the login.
- **Not in this build:** a `.vault` export writes a link as an entry with its fields and no value, and an import skips one as an entry with no value (B5 changes the import and adds the links-only export; this line first said the export left links out, which was wrong); KeePass export and import do not carry `link`, `readers`, `rotated` or `copies` (they do since the follow-ups at the end); linked entries do not default to a Vault named Fleet (B4).

## As built (B4)

Agent2, 2026-11-15, task #47648. Fleet on the batch card, as §3.1 and §4.2 say, with these choices made while building:

- **The card says who may use the Vault.** Under "saved to the Vault <name>" it says one of: "Every agent can use what is in that Vault.", "No agent can use what is in that Vault: only you.", or "Only these agents can use what is in that Vault: <ids>." A named list with nobody in it reads as "only you", as §3.1 says of a new Fleet.
- **A batch of links goes to Fleet when left to itself.** A batch whose every item is a link, and which names no Vault, goes into the Vault named Fleet when the person has made one the asking agent may use. Otherwise, and for any batch that copies a value, it goes into the agent's first Vault as before. A Vault the batch names is the one it goes into. Nothing makes a Fleet: with none, or with one this agent is not in, the batch behaves as it did.
- **The person may choose another Vault on the card.** The card offers the Vaults the asking agent may use, the batch's own among them, in the person's order, and the line about who may use it follows the choice. It offers only those: a Vault the agent is not in can't hold what the agent asked to save, and the card must not tell the agent's pane of a Vault it can't see. With one usable Vault there is nothing to choose and no control is shown.
- **A batch that replaces a value offers no choice.** A row that replaces an entry's value stays with its entry, and a batch is one Vault, so such a card has no control, and an answer that names another Vault for it is refused.
- **The choice is checked again as the same batch.** An Allow with another Vault runs every check of the batch for that Vault (names taken there, the agent's use of it) under the same batch id and references. When it can't go there the answer says so and nothing is copied; the history has the batch as failed. The answer to the agent names the Vault the entries went into.
- **The Fleet filter is the pane's Vault filter.** With more than one Vault the pane already filters by Vault (VS1), so Fleet is one of its choices and there is no second control. The "Readable outside the Vault" badge was built with B2.
- **In an agent's pane the rows come first** (2026-11-16, task #47789). A card has a share of its pane's height, and in a narrow pane at a large zoom the lines about the account and the Vault filled it: the answers were in view and the rows they allow were not, without scrolling the card. Two changes, for a batch's card in an agent's pane only. The rows and their ticks are the first thing under the title; the count of what is ticked, the account, the Vault and its choice follow them. And the card is one scroll box above its answers: the title scrolls away with the rows, and Deny and Allow stay in view. Before, only the text between the title and the answers scrolled, so where the title and the answers filled the card's share of the pane (a 360 px pane at 200%) no row could be reached at all. The same question in the Vault pane's list, which has the pane to itself, keeps the account and the Vault first. Other cards are unchanged. Tests: the order in each place, and the stylesheet's rule.

## As built (B5)

Agent2, 2026-11-16, task #47648. The links-only export of §6's R1, with these choices made while building:

- **"Linked entries only" is a choice in the export dialog**, shown when the Vault has a linked entry. It lists the linked entries alone, each with its tick, and asks for a passphrase as any `.vault` export does. The file is an ordinary `.vault` file (docs/api/VAULT_FILE_FORMAT.md) whose entries are links: their place, who reads the key, the date recorded and the other places. No value is in it.
- **srv keeps the promise, not the dialog.** The host passes `links_only` on, and srv writes the linked entries among the ones chosen and leaves every entry that holds a value out, even one named. With no linked entry it writes nothing and says so. A linked entry found holding a value (`verify` calls that wrong) stops the export. No other format takes the option: a KeePass, CSV or JSON file has no place for a link.
- **A whole export carries links too**, as it did since B2: a link is written with its fields and no part. (B2's notes said it was left out; that line is corrected.)
- **An import takes a link.** An entry whose fields say `link: only` comes in as a linked entry: it needs its place and must hold no part, or it is skipped with the reason shown. It goes to the Vault the person chooses in the import dialog, under its own id when that is free. Until this build a link in a file was skipped as an entry with no value.
- **A link is known again by its place.** On import a linked entry duplicates one in the Vault when the kind and the place match (`source_kind`, `source_location`, `source_key`, `source_region`), so importing the same file twice doubles nothing. "Replace" takes what the file says of the key (readers, date, other places); the entry stays a link, in the Vault it is in.
- **When a link was last checked is not carried.** The check's result is in the history, which belongs to one machine: on the other machine the link reads "not checked yet" until it is checked there.
- **The history line** of a links-only export says how many linked entries were written and that no value was.

After the macOS and Linux look checks of B5 (task #47784), where a link was still worded as a value:

- **The export answer says what was written.** srv answers `links_only` with the count and the file's name, and the dialog says "no value is in the file" on that word, not on what it asked for. An answer without it (a part of the app that does not know the option) is a whole export: the dialog then says the file is a full backup and to delete it if no value was wanted.
- **The import preview marks a link.** Each listed entry says whether it is a link, and shows the Linked badge. The preview also counts how many of the duplicates are links, and the line under "already in your Vault" says a link is the same by its kind and place.
- **Deleting a link** says that the record of where the key lives is removed and the key itself is not touched.
- **The export dialog's formats** start in one column, and in a narrow pane what a format is goes under its name.
- **Dialogs and the pane's zoom.** A Vault dialog follows the app's chrome zoom, as every dialog does, and not the pane's own zoom. That is the app's rule for dialogs, not a Vault choice.

## As built (follow-ups to B2 and B3)

Agent2, 2026-11-16, task #47765. From the review and look checks of B2 and B3:

- **A KeePass database carries a link.** A whole export to KeePass writes a linked entry as an entry with no secret field: its place, and "Read by", "Last changed" and "Also kept at" as unprotected custom fields, and a field "Linked entry kind" that says it is a link and of which kind. The import reads the same names, so the entry comes back a link of that kind, with what it says of the key. An entry that has since been given a password or a secret field in the other app is not a link any more and comes in as what it now is. An entry that holds a value keeps "Read by" and "Also kept at" across the round trip too. The "Linked entries only" choice is still for `.vault` files alone.
- **A batch of links only says "recorded".** Such a batch reads and copies nothing, so its history line says where N keys live and how many were recorded, and its refusals say nothing was recorded. A batch with at least one value to copy reads as before.
- **The history names an AWS place one way.** An import's line names its source as `aws-secret:<id>#<key>`, as a link's line and a check's line do.
- **The machine copy line of an AWS place** says the region and the profile when the entry names them.
- **Each other place says what its newest check found.** On the entry's "Also kept at" line every place is followed by its newest check as the entry is now, "not checked yet", or "can't be checked from here" for a place no check can describe (until now that was only in the answer the asking agent got). A place found changed or missing, or one that could not be read, is marked.
- **A batch of both kinds says how many of each.** When a batch has rows that are read and rows that are links, the card says above the rows how many of the ticked ones are read and copied and how many are only recorded, and the line follows the ticks.
- **The card for a change to what a link says shows one thing a line.** Who reads the key, when it last changed and where else it is kept each have a line, in what is recorded now and in what is asked for, so the date is not at the end of a long list of readers. In what is asked for, a line that differs is marked "changed", one that was not there "new", and a thing the change takes away is a line marked "removed".

# SPEC: one Vault across a person's machines, by LAN or AgentMux Cloud, several Vaults per person, and the Vault's formal link to AWS Secrets Manager

**Status:** active. Approved. The operator, 2026-11-04, through AgentY: "green light the first 4 tasks (agent retake, agent profile, one command, and shared vault) best recommendations": every decision in section 13 is taken as recommended. Spec task #17014; build #17015. Two kinds of step come back to AgentY for the operator before they are built (section 16): anything that costs money, and any new external account.
**Author:** Agent2-6 (Vault). **Sibling specs:** the one sync layer both use, `SPEC_USER_DATA_SYNC_LAYER_2026_11_01.md` (Naki-2, draft; "the layer" below), and the shared Kanban, `SPEC_SHARED_KANBAN_ACROSS_MACHINES_2026_11_01.md` (#17016, Naki-2). The console these services show in is specced separately (Clamk, #17021).
**Verified on:** `main` at `7eb07b508` (2026-11-01), refreshed at `9a21a0105` (2026-11-04) for the approval prompts, which are now cards in the asking agent's pane (#17126).
**Extends:** `SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md` ("the Vault spec") and `SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md`. Private infrastructure details (the cloud store, our fleet's AWS layout) are in a private companion note in `agentmuxai/agentmux-cloud`, not here.

## 1. Summary

The operator, 2026-11-01: "we need to make sure we have a formal relationship bewtween agentmux's vault and aws secets, in fact I just have 2 ideas: 1) Managed vault 2) Managed Kanban .. in both cases, users of AgentMux Cloud can use kanbans and vaults from their own cloud", then "users should still have this ability to shared via the LAN tier, without cloud", then "and then the other question of whether users can have multiple vaults/kanbans". Asked what "their own cloud" means: "their own cloud means an agentmux hosted service". On names: "I think Kanban and Vault will simply retain their names".

This spec makes the Vault work across a person's machines, in three tiers, chosen per Vault:

| Tier | Where the entries are | Needs | Works offline |
|---|---|---|---|
| **This machine** | Only here, as today | Nothing | Yes |
| **LAN** | Every member machine, copied peer to peer on the local network | Machines that are already LAN peers (pinned instance keys) | Yes; machines catch up when they meet |
| **Cloud** | Every member machine, plus a sealed copy in AgentMux Cloud that no one at AgentMux can read | Signing in to AgentMux Cloud | Yes; edits wait and go out when online |

And:
- **A person can have several Vaults** (for example "Personal", "Work", one per project), each with its own tier, its own member machines, its own key and its own list of agents that may use it (section 5). The first build supports this (section 13, decision 1).
- **One sync layer** carries the Vault and the Kanban (section 6). Naki-2 specs it; this spec says only what is particular to the Vault.
- **Values still never reach an agent** (section 9). Sync is srv to srv, everything that leaves a machine is sealed, and no new route returns a value.
- **The Vault gets a formal link to AWS Secrets Manager** for our own fleet: an entry can be backed by a Secrets Manager secret, or mirrored to one, and the spec says which side wins in each case (section 11). Users' Vaults sync only through AgentMux Cloud; there is no user-supplied backend (decided by the operator, 2026-11-01).

The name stays **Vault** everywhere in the app, in the cloud tier and in the console; "AgentMux Vault" only where the brand is needed outside the app.

## 2. What exists today (from the code)

| Area | Today | Where |
|---|---|---|
| Store | One Vault per computer. Tables: `db_vault_entries` (kind, label, `fields` and `tags` in clear, the secret parts in one sealed blob), `db_vault_versions` (earlier values, 7 days; pending rotations), `db_vault_grants` (entry and site; B3, in review, adds `signin:<agent>`), `db_vault_activity` (who did what, never a value), `db_browser_signins`, `db_browser_signin_never`, `db_vault_on_screen` | `crates/srv/src/backend/storage/vault.rs:18-93` |
| Key | One random 256-bit Vault key in the OS keychain (`vault-key`), cached once read; ChaCha20-Poly1305, random nonce, the entry id as associated data, a format byte | `crates/srv/src/backend/vault/seal.rs` |
| Size | A secret part is at most 1 MiB | `crates/srv/src/backend/vault/mod.rs:47` |
| Versions | Kept 7 days; states previous, pending, rotation failed or interrupted | `crates/srv/src/backend/vault/versions.rs:29` |
| Approvals | A use the person hasn't granted is asked as a card in the asking agent's pane, or in the Vault pane's Requests list when no pane on screen can ask it, with srv's deadline (an API key, sign-in or note goes ahead once after 30 s of silence; an ID or card entry waits for a yes); the answer is held in memory, the outcome is a line in Activity | `crates/srv/src/backend/vault/uses.rs`, `server/vault_ask.rs`, `SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md` |
| Export | `.vault` file: entries in clear JSON, encrypted with age to a passphrase; import from 15 other managers | `backend/vault/transfer.rs`, `backend/vault/import/` |
| Instance identity | Each install has an Ed25519 instance key; its id is a hash of that key. Peers pin keys on first sight, and a pin moves only on a certificate signed by a trusted instance; an install can retire itself with a signed revocation | `crates/common/src/jekt_sign.rs:597`, `SPEC_LAN_KEY_CONTINUITY_2026_10_29.md` (built, 0.60.7) |
| LAN | mDNS discovery; srv-to-srv HTTP authenticated by the LAN key; files up to 50 MiB between peers | `backend/lan_discovery.rs`, `server/reactive.rs`, `server/jekt_files.rs:28-35` |
| Sealing to an install | A cloud file's key is sealed to the receiving install: an X25519 key derived from its seed, offered and signed, checked against the account's key directory; ephemeral X25519, HKDF-SHA256, AES-256-GCM | `crates/srv/src/server/jekt_seal.rs` |
| Cross-machine sync | None built. Global Memory sync is a spec (per entry, parent version, fast-forward or a conflict copy) | `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_2026_09_20.md` |
| Secrets Manager | `SecretRef::SecretsManager` exists in the identity model but its resolver returns "unsupported" | `backend/storage/identities.rs:47`, `identity/resolver/secret.rs:85` |

Two facts shape the design:
1. **Labels, kinds, sites and user names are in clear in the local database.** That is acceptable on the person's own disk, but none of it may leave the machine readable: a list of someone's banks and sites is itself sensitive.
2. **There is no person-held secret.** The Vault key is a keychain item, and the person never types a passphrase. Sharing must keep that (no master password to remember) while still letting the person decide which machines get in.

## 3. Goals and non-goals

Goals:
- A person's entries are available on each machine they choose, by LAN without any cloud, or through AgentMux Cloud.
- AgentMux Cloud stores only sealed data. AgentMux can't read entries, labels or Activity, can't add a machine, and can't hand a key to anyone.
- Several Vaults per person, each with its own tier, members and agents.
- A lost machine can be removed, and the Vault re-keyed, from any remaining one.
- Our fleet's secrets in AWS Secrets Manager and the Vault have a defined relationship.

Non-goals:
- Sharing a Vault with other people. The design leaves room for it (members are machines, each with its own sealed copy of the key, so a member can be another person's machine later), but the first build shares only between one person's machines.
- A user-supplied cloud backend (the person's own AWS, S3 or a server). Decided out by the operator.
- A web view of entries. The cloud can't open them, and the console shows only counts and machines (section 12).
- A master password. Recovery is an optional printed key (section 8.5).

## 4. Words used here

- **Vault**: a named collection of entries with one tier, one member list and one key. Today's single Vault becomes the person's first Vault, "Personal".
- **Member**: a machine (an AgentMux install, by its instance key) that holds a Vault's key and a full copy of its entries.
- **Vault key (Kv)**: the random 256-bit key that seals a Vault's records for sync. It changes ("a new epoch") when a member is removed.
- **Record**: what the layer carries: one entry, one grant, one Activity line or the member list, sealed with Kv.
- **The machine key**: today's `vault-key`, which keeps sealing entries on this machine's disk. It never leaves the machine and is not Kv.

## 5. Several Vaults

### 5.1 What a Vault is

Each Vault has an id (a UUID), a name, a tier, a member list, an epoch and its key, and an agent list. Every entry belongs to exactly one Vault. Entry ids stay globally unique UUIDs, so every route that names an entry by id (fill, run with secrets, rotate, the B3 sign-in) works unchanged.

Typical use, and the reason the first build supports several Vaults: the person wants some entries never to leave a machine (that machine's own SSH key, a work token that policy keeps on the work laptop) and others on every machine. That takes at least two Vaults, "This machine only" and a shared one, from the first day.

### 5.2 Which agents see which Vault

Each Vault has an **agents** setting:
- **All agents on its member machines** (the default for Personal, which is today's behaviour);
- **Named agents or groups** (for example only the agents of one project);
- **None**: entries the person uses only themselves (reveal, copy, browser fill they approve).

`Vault list` returns only entries of Vaults the calling agent may use, each with its Vault's name. `Vault request` and `generate` take an optional `vault` (by name or id); without it the entry goes to the agent's default Vault, the first Vault on that machine it may use, in the person's order. A use of an entry in a Vault the agent may not use is refused like an unknown id, so an agent learns nothing about Vaults it can't use.

Grants and approvals are unchanged per entry. The Vault's agent list only narrows who may ask.

### 5.3 Browser sign-ins

Each browser profile has a **Vault for sign-ins** setting (default Personal). Saving a sign-in in that profile puts it there; the key button lists the sign-ins of that profile from that Vault. Incognito still uses Personal and always asks (the B2 rule).

### 5.4 Moving an entry between Vaults

The person moves an entry in the Vault pane (drag, or "Move to"). A move is a delete in one Vault and a create in the other, under the new Vault's key, keeping the entry's id, versions and grants. Activity records it in both. An agent can't move entries: a move can widen who sees a value (into a Vault with more machines or more agents), which is the person's to decide, like a reveal.

### 5.5 Creating and deleting Vaults

The person creates a Vault in the Vault pane with a name and a tier (This machine, LAN, Cloud). Deleting one asks to move its entries elsewhere or delete them; deleting a shared Vault deletes it on every member (a tombstone for the Vault itself), and its cloud copy if it has one.

## 6. Sync: the layer and the Vault's rules on it

The layer replicates **collections** of records, each with a stable id and a hybrid logical clock (HLC) version, with tombstones, a merge rule per collection (an *adapter*), three transports (local only, LAN, AgentMux Cloud) and offline catch-up by version vector. A Vault is one collection, with `join_policy` `approve` and never `readable_by_cloud` (the layer's §3.1). The Vault's ten requirements are in the layer as V1 to V10; this section is what the Vault's adapter does with them. Agent2-6's review of the layer (2026-11-01) asked for the items marked "(asked of the layer)".

### 6.1 What leaves a machine, and in what form

A Vault record on the wire, in a LAN peer's queue or in the cloud is:

| In clear | Sealed with Kv |
|---|---|
| Vault id, record id, record type (entry, grant, activity, members), HLC version, epoch, tombstone flag, size | Everything else: kind, label, fields, tags, expiry, the secret parts, versions, the site of a grant, an Activity line |

The seal is ChaCha20-Poly1305 with a random nonce, and associated data `agentmux-vault-sync-v1:<vault id>:<record id>:<version>:<epoch>`, so a record replayed under another id, Vault, version or epoch doesn't open (asked of the layer). Records over 64 KiB go as sealed chunks (the layer's §3.4); a Vault part is at most 1 MiB. On arrival a member opens it with Kv and stores it as today, sealed with its own machine key; the local tables gain a `vault_id` column.

### 6.2 Source of truth

No machine is primary. Each member holds a full copy, and the latest version of each record by HLC wins, except where 6.3 says otherwise. In the Cloud tier the cloud store holds every record's latest sealed version, so a machine that was off for a month catches up without another member online; it orders records but can't read them. In the LAN tier there is no store: members exchange what the other hasn't seen when they meet, and any member passes on what it holds, so two machines that are never on at the same time still converge through a third.

### 6.3 Conflicts: a value is never merged

When two members change the same entry before seeing each other's change:
- The adapter answers `Conflict(both)` (the layer's §3.3). The version with the later HLC becomes current on every member.
- The other is kept as a version with state **"changed on <machine> at the same time"**, which, unlike other versions, isn't pruned after 7 days.
- The Vault pane marks the entry "Changed on two machines", and it is listed in the Vault's conflict inbox (the layer's §7); the person keeps either. Until then, uses take the current one.
- Tags are merged as a union; other fields follow the value (they describe it).

A rotation in progress (a pending version) is never overwritten by a sync: a record whose base is older than a pending rotation waits as a conflict until the rotation ends. A rotation started on one member marks the entry "rotating on <machine>" for the others, which refuse to start a second rotation of it; the mark lapses with the rotation's own 30-minute timeout.

### 6.4 Deletes and tombstones

A deleted entry becomes a tombstone carrying only its id and version: no value, no label. Each member deletes the entry and its versions when the tombstone arrives. Tombstones are kept until every member has acknowledged them, and at most 90 days: the layer's horizon is a per-collection setting (default 30 days), and the Vault sets 90 (asked of the layer). A member offline longer than that resyncs the whole Vault from scratch: its entries that the others no longer have are shown to the person as "Only on this machine: keep or delete", never silently brought back.

### 6.5 Offline edits

Every member may edit offline. The edits queue on that machine and go out on the next contact, with their HLC versions, and 6.3 settles any clash.

### 6.6 What syncs and what doesn't

| Syncs (in the Vault) | Stays on the machine |
|---|---|
| Entries, their versions and pending rotations, expiry dates | The machine key and the local database |
| Site grants ("Always on this site" for an entry) | Grants to an agent (`signin:<agent>`, a command's first-use grant): they name an agent of this machine |
| Activity (append-only, see 6.7) | `db_vault_on_screen` (which window shows the Vault now) |
| Browser sign-ins and "never for this site", per profile | Pending approvals, requests and capture windows (in memory today, and short-lived) |
| The member list and the Vault's settings | Files written for one command (deleted when it ends) |

Browser profiles are per machine today, so a synced sign-in names its profile by name; on a machine without that profile it shows under Personal, marked with the profile's name.

### 6.7 Activity

Activity becomes append-only per machine: each line gets its machine and a sequence number, so lines from different machines never conflict. The Vault pane shows every member's lines with a Machine column. Lines are sealed records like any other.

## 7. Tiers in practice

### 7.1 This machine

As today. Nothing is sent anywhere.

### 7.2 LAN

Members must be LAN peers whose instance keys this machine has pinned (built: `SPEC_LAN_KEY_CONTINUITY_2026_10_29.md`). Sync runs over the LAN srv-to-srv channel, each request signed by the sender's instance key and checked against the pin, and only to and from members of that Vault. Nothing goes through AgentMux Cloud, and a LAN Vault works with no AgentMux account at all.

### 7.3 Cloud

Members are installs signed in to the same AgentMux account. Records go to the cloud store (sealed), and members on the same LAN also exchange them directly, so a Cloud Vault keeps working between machines in the same house when the internet is down. The cloud refuses records from an install that isn't a member of that Vault, as a second line; the first is that a non-member has no Kv.

### 7.4 Changing a Vault's tier

- **This machine → LAN or Cloud**: the person adds members (8.2).
- **LAN → Cloud**: the existing records are uploaded, sealed.
- **Cloud → LAN or This machine**: members stop sending, and the cloud copy is deleted. The pane says that AgentMux Cloud deletes it, and that deletion is AgentMux's word, not something the machine can check; the data was sealed throughout.

## 8. Keys: joining, removing, recovering

### 8.1 Keys per member

Each member has an X25519 key derived from its instance seed with HKDF, signed by its instance key, as `jekt_seal.rs` already does for an agent (asked of the layer: instance keys are Ed25519 signing keys and can't be sealed to directly). Kv is sealed to each member separately: an ephemeral X25519 key, HKDF-SHA256, then AES-256-GCM, as for a cloud file's key. The member list is itself a record, signed by the member that last changed it, and lists each member's instance id, its X25519 key and the epoch.

### 8.2 A machine joins

1. On the new machine, the person opens the Vault pane and chooses **Join a Vault**. It lists the Vaults its LAN peers offer (LAN tier) or the account's Vaults (Cloud tier), by name and member machines, never contents.
2. Any current member shows the request: "**<machine>** wants to join **Work**. The code on both screens should match: **482 913**." The code is derived from both machines' instance keys and a fresh nonce, so a machine pretending to be the new one, or a cloud that swapped a key, shows a different code (asked of the layer for every `approve` join: LAN pins are first-seen, and the cloud's key directory is the service's word).
3. The person approves on the member. It seals Kv to the new machine's key, adds it to the member list, signs, and the new machine receives the records.

An agent may start a join request (`Vault` `join_request`, naming the Vault), but only the person approves it, in that prompt. A join hands every value in the Vault to another machine, which is the same as revealing all of them, and reveals are the person's (the Vault spec). This guards against a value leaving, not an agent acting, and it is the one person-only step this spec adds (section 13, decision 2).

### 8.3 Joining with entries already there

A machine that already has its own Personal Vault keeps it, renamed "Personal (<machine>)", tier This machine. The pane offers to move its entries into the shared Vault. Identical entries (same kind, label, site, user name and value) become one; entries that differ only in value are both kept, marked for the person.

### 8.4 Removing a lost or retired machine

From any remaining member: **Remove <machine>** (the person again, as for a join). That member:
1. makes a new Kv (epoch n+1) and seals it to each remaining member;
2. signs a member list without the removed machine, which members and the cloud then use to refuse its records;
3. re-seals every record at the new epoch (the cloud copy is replaced);
4. lists the entries the removed machine held, offering to rotate those that have a rotation recipe.

A removed machine still holds what it had when it was lost: no design can take that back. What it loses is every later change. The same steps run when an install retires itself with a signed revocation (built), without asking, since that install asked.

### 8.5 Recovery

If every member is lost, a Cloud Vault's sealed copy can't be opened: AgentMux never holds Kv. When a Vault goes to the Cloud tier, the pane offers a **recovery key**: 24 words, shown once, that seal Kv (age to a passphrase, like a `.vault` export). With it, a new machine signed in to the account opens the cloud copy. Without it, nothing can. In the member record it is a "recovery" member: no machine, never sent records, only able to unseal Kv (asked of the layer). The console shows whether a recovery key was made, never the key itself.

## 9. Values never reach an agent

The invariant is unchanged: **an agent directs a value; it never holds one.**
- Sync is srv to srv. The layer has no agent-facing read path, and no new MCP action or route returns a value or a sealed record.
- A value from another machine is used on this one like a local one: a fill, a command's environment or a sign-in, each with today's approval and scrubbing.
- An agent's `Vault list` is filtered by the Vault's agent list (5.2).
- The relay, the cloud store and LAN peers that aren't members see only the clear columns in 6.1.

## 10. Approvals, audit and container agents

- **Approvals** stay on the machine where the agent runs: its card shows in that agent's pane there, and that machine's Vault pane lists it. Showing a prompt on the person's other machines, or on the phone, is a later step; it needs a signed approval message, not this sync.
- **Audit**: Activity from every member is in every member's Vault pane (6.7). A use on machine B of an entry made on A appears on both once synced.
- **Container agents** use entries exactly as today, through srv's routes on their host, with the Vault's agent list applying to them as to host agents. A container is never a member, never holds Kv, and takes no part in sync: its host's srv does.

## 11. The Vault and AWS Secrets Manager

### 11.1 Our fleet

Our fleet keeps shared secrets (service keys, signing keys, tokens) in AWS Secrets Manager, read by hosts with their AWS identity. The layout and policy are in the private companion note. The formal link is a per-entry **backing**:

| Backing | Authoritative | What the Vault does | Editing |
|---|---|---|---|
| **None** (default) | The Vault | As today | In the Vault |
| **Backed by Secrets Manager** (`aws-sm:<secret>#<json key>`) | Secrets Manager | Reads the value from Secrets Manager at each use, with the host's AWS identity, and keeps no copy; the entry has a label, kind and grants like any other | In Secrets Manager. The Vault pane shows "Managed in AWS Secrets Manager" and has no Edit |
| **Mirrored to Secrets Manager** | The Vault | After each change or rotation, writes the new value to Secrets Manager under the host's escalation for writes, giving its reason | In the Vault; a write to Secrets Manager outside the Vault is overwritten at the next rotation, and the pane shows the last mirror time and result |

Rules:
- One resolver reads Secrets Manager for both the identity model's `SecretRef::SecretsManager` (unimplemented today) and backed entries.
- A backed entry is never synced as a value: other members get the backing reference and read Secrets Manager themselves, if their host's identity can.
- **The Vault hides a value only from agents that can't read it elsewhere.** An agent whose own cloud identity can read that secret in Secrets Manager isn't stopped by the Vault. Such entries are marked "readable outside the Vault" in the pane, and the private note says which fleet secrets this applies to and what would close it.
- Approvals and Activity apply to backed entries as to others: a use through AgentMux asks and is logged even though the value lives in AWS.

### 11.2 Users

Users' Vaults use only AgentMux's own tiers (decided by the operator). AgentMux Cloud's store is AgentMux's to choose. It holds sealed records, so it gains nothing from Secrets Manager's own encryption. The private note records that choice.

## 12. What the Vault needs from the console

The console (Clamk's spec, #17021) shows AgentMux Cloud's services. For the Vault it shows only what the cloud can know:
- **Vaults**: name, tier, member count, entry count, size, last change. Names are the one readable item, and only if the person chooses "Show Vault names in the console" when creating the Vault; otherwise "Vault 1", "Vault 2".
- **Members** of each Vault: machine name, last seen, epoch, and "Remove". The console can't re-key (it has no Kv), so Remove there marks the machine "to be removed": the cloud stops serving it at once, and the next member to come online does 8.4 and asks the person to confirm.
- **Storage and limits**: bytes per Vault, the account's quota.
- **Recovery key**: made or not, and when.
- **Delete a Vault's cloud copy.**
- Never entries, labels, sites, user names or Activity: the cloud can't read them.

## 13. Decisions

All six were taken as recommended (the operator, 2026-11-04, "best recommendations").


1. **Several Vaults in the first build, not just room for them.** Decided: yes, in VS1, before any sync. The first sharing request will be "share these, not those", which needs two Vaults; adding them after sync ships means migrating every member. Entry ids are already global, so no agent route changes. (The layer's §9 recommends one of each kind first; for the Kanban that stands, and the layer itself carries a collection id everywhere either way.)
2. **Who approves a join and a removal.** Decided: the person, with the matching code (8.2). An agent may ask. This is the one person-only step added, because a join is a reveal of every value. It is not a gate on agents acting (an agent starts, follows and is told the outcome of a join): it is the Vault's invariant, an agent directs a value but never holds one, applied to a machine. An agent that could approve a join could send every value to a machine it controls.
3. **A recovery key for Cloud Vaults.** Decided: offered, optional, shown once (8.5). Without one, losing every machine loses the Vault.
4. **Sharing with other people.** Decided: later. The member model already allows it.
5. **The fleet's default backing for secrets that cloud services read.** Decided: Backed by Secrets Manager (AWS authoritative) for secrets that services and Lambdas use; Mirrored for secrets the Vault creates or rotates and a service then needs.
6. **Tombstone horizon for Vaults.** Decided: 90 days (6.4), against the layer's 30-day default for boards.

## 14. Migration

- Today's Vault becomes **Personal**, tier This machine, with id `personal` on each machine. The local tables gain `vault_id` (default `personal`). The machine key and every sealed row stay as they are: nothing is re-sealed, nothing leaves.
- Grants, versions, Activity and sign-ins get the same `vault_id`.
- Nothing is shared until the person creates a shared Vault or turns Personal into one, and adds a machine.
- Two machines that both have entries are joined as in 8.3, never merged silently.
- `.vault` export gains the Vault's name and can export one Vault or all of them; import asks which Vault to fill.

## 15. Who builds what

As agreed with Naki-2 (the layer's §12):
- **The layer** (its S0 to S1: change log, HLC, the adapter trait, the LAN transport, membership, keys and the join flow; its S3 client side of the cloud tier): Naki-2. Agent2-6 reviews it for the Vault's needs.
- **The Vault adapter** (the layer's S2: sealed records, keep-both, versions, Activity as append-only, epochs and re-sealing), and the Vault-only phases below (several Vaults, the recovery key, the Secrets Manager link): Agent2-6.
- **The managed store and its API** in `agentmuxai/agentmux-cloud`, one store for Vaults and Kanban boards: AgentY's lane (relay and MuxBus), specified privately.
- **The console's Vault pages**: Clamk, from section 12.

## 16. Phases

Named VS (Vault sync) so they don't clash with the layer's S0 to S4.

| Phase | What | Depends on |
|---|---|---|
| **VS1** | Several Vaults on one machine: `vault_id`, the picker, the agent list per Vault, Move to, a sign-in Vault per profile | Nothing |
| **VS2** | LAN tier: the Vault adapter (the layer's S2), the join with a code, sync of entries, site grants and Activity, the conflict inbox, tombstones, removal and re-keying | The layer's S0 and S1 |
| **VS3** | Cloud tier: upload and catch-up through the store interface, the recovery key, the console's Vault pages | The layer's S3, the managed store, the console |
| **VS4** | Secrets Manager: the shared resolver, Backed by, Mirrored to | Nothing (can run beside VS1) |
| Later | Other people as members; approvals on another machine or the phone | VS3 |

One PR per phase; VS1 and VS4 can start before the layer is ready.

**As built (VS1a, the Vaults themselves, srv and the Vault tool).** VS1 is two PRs: this one, then the pane (the picker, Move to, the agents setting, a sign-in Vault per profile).
- `db_vaults` (id, name, `agents` = all, none or named, `agent_ids`, position) with Personal seeded, and `db_vault_entries.vault_id` (default `personal`). Grants, versions and Activity follow their entry for now and do not carry their own `vault_id`: on one machine the entry is the key, and the sync phases add the column when a record has to travel alone.
- `backend/vault/vaults.rs`: create, rename and set agents, delete when empty (Personal stays), `move_entry` (the person's, recorded in Activity as "Personal -> Work"), `may_use`, `default_for` and `resolve`. A name is unique without case, at most 60 characters.
- An agent's access: `Vault list` (now `POST /api/v1/vault/list`, carrying its signed identity; the old `GET` answers as an agent that proves nothing, so it sees only Vaults open to all agents), `generate`, a web-form fill, a command's `secrets` and a capture all go through the entry's Vault. An entry in a Vault the agent may not use is refused with the same message as one that does not exist, and `resolve` lists only the Vaults the agent may use. A named agent is matched by id without case; named groups are not built yet.
- `request` and `generate` take `vault` (name or id). `generate` makes the entry there, or in the agent's first Vault; `request` records the Vault on the request so the form can start on it.
- Host ops: `vault_create`, `vault_update`, `vault_delete`, `move_entry`; `list` returns `vaults` and each entry's `vault_id`; `create` takes `vault`.
- An older AgentMux on the same computer shares the identity store and does not know the column: it lists every Vault's entries, ignoring the agent lists. Nothing is exposed that was not already on that machine, but the agent list is enforced only by builds from this one on.
- Import still fills Personal; asking which Vault to import into comes with the pane.

**As built (VS1b, the pane).** VS1 is done with this one.
- A Vault picker above the sections (once there is more than one Vault) narrows every list to one Vault or all; the choice is kept in the pane's meta (`vault:vault`). A **Vaults** button opens the dialog: add, rename, set agents (all, named by id, none) and delete an empty Vault. Named agents are typed as ids, separated by commas, as the Swarm shows them; a list to pick from comes later.
- **Move to** on each entry (when there is more than one Vault) says which agents may use it in the Vault it goes to. The new-entry form and Import ask which Vault when there is more than one; a form opened from an agent's request starts on the Vault it named. Import's new entries go to the chosen Vault, and an entry it replaces stays in the Vault it is in (before, it went back to Personal).
- **A sign-in Vault per browser profile**, in the Vaults dialog (Personal and each of the person's profiles; an agent's own profile is not listed). `db_vault_signin_vaults` holds the choice (Personal keeps no row), because the Personal jar has no entry in `browser-profiles.json`. A profile saves new sign-ins in its Vault and offers, fills and lists only the sign-ins kept there, for the key button and for an agent's `Browser` `signin` alike. Sign-ins already saved stay where they are: moving a sign-in out of a profile's Vault takes it out of that profile's tabs, and the dialog says so. Deleting a Vault sends the profiles that used it back to Personal. Incognito still uses Personal.
- Host ops: `signin_vault` (profile, vault_id); `list` returns `signin_vaults`; `import_commit` takes `vault`.
- Not built: the Vault's tier and members (VS2, VS3), and dragging an entry to a Vault.

**Order and what comes back first** (2026-11-04). VS1 is built first: it is local, costs nothing and needs no account. VS4's resolver and **Backed by** reads come next; they read secrets the fleet already has, with the hosts' existing identities. Two steps go back to AgentY, for the operator, before they are built, because they cost money:
- **Mirrored to Secrets Manager** creates new secrets in our AWS account, each a monthly charge.
- **VS3** needs the managed store in AgentMux Cloud (storage and requests, in `agentmuxai/agentmux-cloud`).

VS2 waits on the layer: its spec (`SPEC_USER_DATA_SYNC_LAYER_2026_11_01.md`, Naki-2) is a draft not yet approved, and its S0 and S1 come after Naki-2's current tasks. No step needs a new external account.

## 17. Trust model and security

Who is trusted with what. "Reads" means the values and the labels; everyone else sees at most the clear columns of 6.1.

| Party | Trusted to | Not trusted to | What enforces it |
|---|---|---|---|
| **The person** | Decide which machines are members (join, remove), reveal, move an entry between Vaults, answer a use | | The prompt is drawn by the host, operator-only, answered through the host's route; the join code is checked by eye on two screens |
| **A member machine** (its srv) | Read and write the Vault's records; seal Kv to a new member the person approved; re-key on a removal | Keep a value from the person; add a member on its own | Kv sealed to its X25519 key; every record signed into the member list's epoch; Activity names the machine that wrote each change |
| **An agent** (host or container) | Direct a value: a fill, a command's environment, a sign-in, each with today's approval; ask for an entry; start a join request | Hold a value; approve a join or a removal; move an entry; see a Vault outside its Vault's agent list | No route returns a value; the agent list filters `Vault list` and refuses a use like an unknown id; joins and moves are host-only actions |
| **A LAN peer that isn't a member** | Carry nothing for the Vault | Learn more than that a Vault exists, its id and record count; join | No Kv; sync answers only members, checked against the pinned instance key |
| **AgentMux Cloud** (the relay and the store) | Store and forward sealed records; order them; stop serving a machine marked for removal | Read records, labels or Activity; add a member; hand out Kv; roll a member back undetected | Records sealed with Kv, which the cloud never has; the join code; associated data with id, version and epoch; members refuse older versions and old epochs |
| **AgentMux the company** | Run the cloud as above, and delete a cloud copy when asked | The same as the cloud: AgentMux can't open a Vault, and says so in the pane | The same; deleting a cloud copy is AgentMux's word, which the pane says |
| **AWS Secrets Manager** (our fleet only) | Hold the value of a Backed entry, which the Vault reads at each use; receive a Mirrored entry's value | Be bypassed by the Vault's approvals | Each use through AgentMux is approved and logged; an agent whose own cloud identity can read the secret is outside the Vault's protection, which the pane marks (11.1) |

The invariant across every tier: **values move only between the person's own member machines, sealed, and reach an agent's command, form or sign-in only through an approved use.**


- **What a LAN peer that isn't a member can learn**: that a Vault exists, its id and its record count, nothing else. It can't join without the person's approval on a member.
- **What AgentMux Cloud can learn**: the clear columns of 6.1 (how many records, their sizes, when they change), the member machines, and Vault names only if the person allowed them. It can't add a member, because a member needs Kv sealed by another member and the person's matching code.
- **A stolen machine**: has its copy as of its last sync, protected at rest by the OS keychain as today. Removing it re-keys (8.4).
- **Replay and rollback**: the associated data binds id, version and epoch; a member refuses a record older than what it holds, and records from an old epoch after a re-key.
- **A compromised member**: can read the Vault (it is a member) and write bad records, which every member applies. Activity shows which machine wrote each change, and versions keep 7 days of earlier values.

## 18. Tests

- Sealing: a record opens only with its Vault, id, version and epoch; a non-member's copy doesn't open.
- Conflicts: two offline edits leave one current value and one "changed at the same time" version on both members; a rotation in progress isn't overwritten.
- Tombstones: a delete removes the entry and its versions on every member; a member back after the horizon gets "Only on this machine", not a resurrected entry.
- Join: a wrong code isn't accepted; an agent's join request waits for the person.
- Removal: after a re-key the removed machine's records are refused and new records don't open with its old key.
- Agents: `Vault list` hides Vaults the agent may not use; a use of their entries is refused like an unknown id.
- Secrets Manager: a backed entry reads at use and keeps no copy; a mirror write gives its reason; neither path returns a value to an agent.

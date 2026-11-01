# SPEC: one sync layer for a user's own data: Kanban boards and Vaults across their machines, by LAN or AgentMux Cloud

**Status:** draft, 2026-11-01. Written for tasks #17016 (Kanban, Naki-2) and #17014 (Vault, Agent2-6), who agreed that Naki-2 owns this layer spec and Agent2-6 reviews it and writes only the Vault's part. Not yet approved by the operator.
**Author:** Naki-2@narko, with Agent2-6's ten constraints for the Vault (§3, each marked "V").
**Trigger (operator, 2026-11-01):** "I just have 2 ideas: 1) Managed vault 2) Managed Kanban .. in both cases, users of AgentMux Cloud can use kanbans and vaults from their own cloud", then "users should still have this ability to shared via the LAN tier, without cloud.", then "whether users can have multiple vaults/kanbans", and "their own cloud means an agentmux hosted service".
**Naming (operator):** "Kanban and Vault will simply retain their names." In the app, the console and the cloud tier they are "Kanban" and "Vault", with no Mux- prefix; "AgentMux Vault" only where the brand is needed outside the app.
**Verified on:** `main` at `e4f374c61` (2026-11-01), AgentMux 0.60.10.
**Companion specs:** `SPEC_SHARED_KANBAN_ACROSS_MACHINES_2026_11_01.md` (the Kanban's part), and the Vault's part in Agent2-6's #17014 spec. **Builds on:** `SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md` (the LAN proof, `Op`, the `host` argument) and its §4.6 (the fleet Global Memory sync, the precedent).

## 1. Summary

1. **A collection is the unit.** A Vault or a Kanban board is one *collection*. Each collection has its own members (machines now, other people later), its own tier, its own key and its own merge rule. A user can have several of each. The first build ships one Kanban board; Vaults ship several from their own first phase, locally, before any sync (§9).
2. **Three tiers, additive.** *Local*: one member, no network, as today. *LAN*: the user's own machines sync peer to peer, with no cloud, over the proof they already have. *AgentMux Cloud*: the same collection also syncs through a hosted AgentMux service that stores and relays records. Cloud is **AgentMux Cloud only** (the operator: "their own cloud means an agentmux hosted service"); a user's own backend is not a tier. The one exception is the fleet's own link from the Vault to AWS Secrets Manager (#17014), which is a Vault feature, not a tier.
3. **The local store stays the source of truth.** The Kanban keeps its tables, the Vault its entries. The layer sits beside them: a log of changes, a replicator, and an *adapter* per kind of collection that says how to read a record, how to merge one and how to write the result back.
4. **Every change is a versioned record**, ordered by a hybrid logical clock, shipped as an opaque (sealed) blob with a few clear header fields. Merging is the collection's rule, never the layer's guess: field-by-field newest-wins for the Kanban's content, keep-both for the Vault's secrets, append-only for histories, and a single authority per task for the Kanban's claims.
5. **Sync runs in srv on the host only.** Container agents take no part and see nothing of it (V8). The layer gives agents no read path: a Vault value reaches an agent only through today's use and approval routes.
6. **Who builds what** (§12): the layer core, the LAN transport and the Kanban adapter: Naki-2. The Vault adapter and the Vault's key use: Agent2-6. The managed service's side of the cloud tier: AgentY's lane, specified privately; this spec says only what the client needs from it (§8).

## 2. What exists

- **The LAN proof and the `host` argument** (remote ops): a machine proves itself to a LAN peer with the per-agent and instance keys pinned on first use; a request is signed, carries its audience, op, params hash, time and nonce, and is refused when unproven (`remote_ops/receive.rs`). `tasks.enqueue` and `tasks.list` (`work_remote.rs`) and the Global Memory ops (`remote_ops/memory.rs`) already run over it.
- **The fleet Global Memory sync is the precedent** (`remote_ops/fleet_sync.rs`, `backend/fleet_memory.rs`, spec §4.6 of the remote-ops spec): one loop every 15 seconds fingerprints what this machine holds for the fleet; for each proven same-account LAN peer whose last sync had another fingerprint, it lists what the peer holds and sends what the peer lacks or has older; a peer that is offline, unproven or without the ops is left alone until a retry time (5 minutes), with a full resync every 10 minutes to heal a lost push. Last write wins. It is LAN-only, whole-collection, and has no tombstones, no per-record versions beyond the entry's own history, and no keys. The layer is its generalization: per-record versions, deletions, partial catch-up and a second transport.
- **The Kanban** (`backend/storage/work_queue.rs`, `work_board.rs`, `work_deps.rs`): `db_work_queue` (UUID `id`, `state`, `claimed_by`, `claimed_by_uid`, `claim_expires`, `attempts`, `target_agent`…), `db_work_cards` (`num`, an `INTEGER PRIMARY KEY AUTOINCREMENT`, **per machine**; `rank`, `review_required`, repo, PR, checklist…), `db_work_events` (an append-only history), `db_work_deps`. All in the one identity store of a machine, shared by every AgentMux on it.
- **The Vault** (`backend/vault/*`, `storage/vault.rs`): entries sealed one by one with ChaCha20-Poly1305 under one local key in the OS keychain (the entry id is the associated data), earlier values kept as versions (`versions.rs`), an append-only activity log.
- **AgentMux Cloud, client side:** the machine holds an account token and keeps a WebSocket to the hosted relay (`cloud_subscriber.rs`); every machine signed in to one account is one sync group. Nothing of that carries a user's collections today.

## 3. The model

### 3.1 Collections, members, keys

A **collection** is `{id, kind (kanban | vault), name, tier (local | lan | cloud), members, join_policy, key epoch, readable_by_cloud}`.
- **Members** are machines, each named by its pinned instance key (a person's other accounts, for sharing with other people, are the room §9 leaves). A **recovery member** is allowed too (Agent2-6): no machine, never sent records, only a recovery key (24 words, optional, offered for cloud Vaults) that also seals the collection key, used to unseal on a new machine. The member list is itself a record of the collection, signed by an existing member.
- **The collection key** is 256 random bits, made on the first machine and handed to each member when it joins. An instance key is an Ed25519 *signing* key, so the key is not sealed to it directly: each instance has an X25519 key derived from its seed with HKDF and signed by the instance key, and the collection key is sealed to that with an ephemeral X25519 key, HKDF-SHA256 and AES-256-GCM. That is exactly `server/jekt_seal.rs` (built for cloud file keys), which the layer reuses. Records leave srv sealed with the collection key (V2): ChaCha20-Poly1305, with the **collection id, record id, version and epoch** as associated data, so an old version can't be replayed under the same id and a record can't be moved into another collection. The clear header holds only: collection id, record id, hybrid-clock version, origin machine and sequence, key epoch, tombstone flag, size. **A Vault's labels, kinds, sites and user names are inside the seal** (V2), so a LAN peer's logs and the relay see nothing readable.
- **`join_policy`:** `auto_proven` (a machine already proven by the LAN trust and signed in to the same account joins at once: the Kanban's default, as the fleet Global Memory does) or `approve` (a person approves on an existing member, in the host window: the Vault's default, V4). **An approval shows a short code on both screens**, derived from both instance keys and a fresh nonce, which the person compares: LAN pins are first-seen and the cloud's key directory is the service's word, so without the code a rogue LAN install or a swapped key would be handed the collection key. An agent may *start* a join request; only the person approves it.
- **Removing a machine** moves the collection to epoch n+1: a new key, sealed to the remaining members; every record is re-sealed in a background pass; the layer refuses to send to a removed member (V4). The same runs **automatically when an install retires itself with a signed revocation** (built, LAN key continuity). The cloud keeps the old epoch's records until the re-sealed set is complete, so a crash in the middle of the pass loses nothing.
- **`readable_by_cloud`:** a collection whose records the hosted service may read (§8.2): off for every Vault, always; a choice for a Kanban board.

### 3.2 Records and versions

- **A record** has a stable `record_id` (a UUID made where the record was born; never reused) and a **version**: a hybrid logical clock `(wall milliseconds, counter, machine id)`, totally ordered. A machine that sees a version from the future beyond a skew limit (5 minutes) refuses it and says so in the audit, so a machine with a wrong clock can't win every merge.
- **Every machine numbers what it originates** with a per-collection, per-machine sequence. A catch-up is "send me everything from origin X after sequence n, for each origin I know": a version vector, so it is exact, resumable, and nothing needs rescanning.
- **Three record shapes**, chosen by the adapter:
  - **Value:** one sealed body and a version (a Vault entry; a Kanban card's lifecycle).
  - **Fields:** a map of named fields, each with its own version (a Kanban card's content). Merging is per field, newest wins, so two people editing different fields of one card both keep their edit.
  - **Append:** immutable entries keyed `(origin machine, sequence)` (the Kanban's event history, the Vault's activity and use records). They can't conflict (V6).
- **A tombstone** is a record with no payload (V5): its id, version and epoch. A deleted secret's value does not live on in it. It is kept until every member has acknowledged it, or until a horizon (a per-collection setting: 30 days by default, 90 for a Vault); a member that returns after the horizon resyncs that collection from scratch, and anything it changed offline goes to its **conflict inbox** (§7), not silently lost.

### 3.3 Merging belongs to the collection

The layer calls the adapter with `(local version, remote version)` and gets back `Applied`, `Kept local`, or `Conflict(both)`. It never decides what a conflict means.
- **No silent last-writer-wins for secrets** (V3): the Vault's adapter answers `Conflict(both)`, keeps both as versions (the existing 7-day versions), and the person picks.
- **Field-wise newest-wins for the Kanban's content**, with the overwritten value kept in the card's event history ("title changed by Lazo@Area54, was …").
- **A single authority for the Kanban's lifecycle** (claims, state, attempts): see the Kanban spec §4. The layer doesn't merge those; it carries the authority's decisions.

### 3.4 Blobs (V7)

A record body over 64 KiB, and every Kanban media file and Vault file entry, goes as **content-addressed chunks** (256 KiB, named by their hash, sealed like records). The record holds the list of chunk hashes and the total size; chunks are fetched on demand from any member that has them (LAN) or from the relay (cloud), and a size cap per record is a setting of the collection (default 25 MiB; the console shows usage). A card shows a placeholder until its media arrives.

## 4. Transports

### 4.1 Local

A collection with one member. The change log is still written (so adding a second machine later needs no migration), and nothing is sent.

### 4.2 LAN

New remote ops, over the existing proof, each `Op::write` on the receiver and each refused unless the sender is a member of the named collection:
- `sync.hello {collection, epoch, vector}`: who I am, my key epoch, and the version vector of what I hold. The receiver answers with its own and, when the epochs differ, says so.
- `sync.pull {collection, since: {origin: seq}, limit}`: records after those sequences, in batches of at most 64 KiB.
- `sync.push {collection, records}`: records the peer lacks (idempotent: applying a record by `(record id, version)` twice changes nothing).
- `sync.chunk {hash}`: a blob chunk.
- `sync.members {collection}`, `sync.join {…}`: the join flow of §3.1.

**The loop** keeps the fleet sync's shape (a tick every 15 seconds, no change observer) with two upgrades: a change wakes the loop at once (a push to every reachable member within a second), and catch-up uses the version vector instead of a whole-collection fingerprint. A peer that is offline, unproven or without the ops is left alone until a retry time. Nothing is sent when nothing changed, so the audit trails carry only real work.

### 4.3 AgentMux Cloud

The same records and the same version vectors, through a store interface (§8): `put(collection, records)`, `get(collection, since vector)`, `chunk(hash)`, and a wake notification on the connection the machine already keeps. A cloud collection syncs over **both**: a peer on the same network is reached directly (fast, no round trip through the service), and every member also writes to the service, which is how a machine that is never on the same network as another still converges. A user who turns the cloud tier off leaves LAN sync as it was.

## 5. Offline, conflicts, ordering

- **Offline machines** keep working: writes go to the local log and the local tables as today. On the next contact each side sends what the other lacks (§3.2's vector). A short outage costs nothing; a long one is covered by the tombstone horizon (§3.2).
- **Ordering:** records apply in version order per record; a record that depends on another not yet arrived (a task that starts after one the machine hasn't received) is held and applied when it does; the adapter sees it as the predecessor not yet known.
- **Duplicate and replayed delivery** is harmless: apply is idempotent.
- **A partition** where two machines both change one field resolves to the newer version; both stay in history.
- **Clock skew** (§3.2) is bounded by refusal; the audit names the machine.

## 6. Security

1. **The relay and LAN peers see opaque records.** Clear header fields: collection id, record id, version, origin and sequence, epoch, tombstone flag, size. For a collection with `readable_by_cloud`, the hosted service holds the key and can read what it stores (§8.2); for a Vault, never.
2. **Membership changes are signed** by an existing member and, for `approve` collections, shown in the host window naming the machine; no agent can answer that window (the same rule as every approval window).
3. **Agents gain nothing.** The layer has no agent route that reads or writes a record. The Kanban's agents use the same `Tasks` tool as today, which writes the local store, and the layer replicates what that wrote; the Vault's agents use today's use and approval routes (V8).
4. **Every remote write is audited** on both machines (the remote-ops audit rows), naming the machine and the collection, never a value.
5. **A removed machine** keeps what it already had (it can't be made to forget) but receives nothing after the key moves to the next epoch, and its pushes are refused.
6. **A tombstone carries no payload** (V5), and a re-sealed pass after a removal covers the Vault's earlier versions too.

## 7. Conflicts a person sees

- **The conflict inbox**, per collection: what a returning member changed that the merge could not place (after the tombstone horizon), and the Vault's keep-both entries. It is a list in the collection's own view, with "keep this" and "keep that"; nothing is dropped without a choice.
- **The Kanban** shows an overwritten field in the card's history; it needs no inbox beyond the case above.

## 8. What the layer needs from the hosted service, and from the console

### 8.1 The store interface (V9)

The client talks to the cloud tier only through: `put` and `get` of sealed records by collection and version vector; `chunk` get and put; a wake notification; membership records stored as ordinary records. The store also refuses `put` and `get` from an install that is not in the signed member list, checking the signature against the account's key directory: a second line, since the first is that a non-member has no key. One store serves both kinds of collection, keyed (account, collection, record). Everything behind it (storage, quotas, the account model) is the managed service's, specified privately. A future backend (including a user's own, which the operator ruled out for now) would implement the same interface; it is not built.

### 8.2 What each collection kind needs from the console (Clamk's specification)

- **Collections list:** name, kind (Kanban or Vault), tier, members (machines, with last contact and online state), sync health (lag, queue length, last error), key epoch.
- **Membership:** add a machine (the approval of §3.1 lands here and in the host window). Removing one **marks it "to be removed"**: the service stops serving it at once, and the next member that is online re-keys (the console holds no collection key and can't) and asks the person; the console then shows the re-seal's progress. Any cloud-side action that needs the key works this way.
- **Tier:** turn the cloud tier on or off per collection, with what it means said plainly (what the service can and can't read).
- **For a Kanban board only:** the "readable by AgentMux Cloud" switch (§11 decision 2). Off makes the service store sealed records it can't show, so the console and the mobile app can list the board's existence and sync state but not its cards.
- **Usage and limits:** blob storage by collection, record counts, the per-collection size cap.
- **Audit:** the remote-op audit rows for the collection, and the conflict inbox.
- **Delete and leave:** a machine leaves a collection (it keeps its copy); the last member deleting a cloud collection asks the service to delete what it holds.

## 9. Several collections per user (the operator's question)

**The model supports many:** every record, version, key and membership carries a `collection id`, and nothing in the layer assumes one per kind. A user could have Vaults "personal", "work", "client X" and boards "main", "this project", each with its own tier and members (this machine only; the LAN machines; cloud; other people later).

**Recommendation: one Kanban board in the first build, with every table and message already carrying the collection id; Vaults ship several, from the Vault's own first phase (VS1 in the Vault spec, #17014 §5), locally and before any sync.** Why the difference:
- Several boards change what `#168` means (the Kanban spec §3.2), so the board waits. Several Vaults don't: entry ids are already global UUIDs, so routes don't change, and the first sharing request will be "share these, not those", which needs a This-machine Vault beside a shared one.
- Which agents see which collection is a policy that needs its own design: proposed, an agent sees a collection if it runs on a member machine **and** the collection's agent list (default: all) includes it; a Vault additionally keeps today's per-use approval for every entry. A use of an entry in a Vault the agent can't see is refused like an unknown id. A container agent sees collections through its host machine's membership, with the same rules.
- Moving a record between collections is a delete in one and a create in the other, re-sealed (V10): there are no cross-collection references. A Kanban task moved to another board gets a "moved" marker in the source and a link back, not a shared identity.

## 10. Migration

Each collection kind migrates its own data (the Kanban spec §8). The layer's migration is: a new `m0045`-style migration adds the change log and state tables beside the existing ones (no change to the Kanban's or the Vault's tables), existing rows are given an origin of this machine and a first sequence in one pass at first start, and a machine with no peers behaves as before (Local).

## 11. Decisions for the operator

1. **Auto-join:** same-account machines already proven by the LAN trust join the *Kanban* at once (like the fleet Global Memory) and a person approves each machine joining a *Vault* (recommended). Alternative: approve every join.
2. **Can AgentMux Cloud read a Kanban board?** *Recommended: the user chooses per board, and a new board is readable by default*, because a managed Kanban is the product (a console view, the mobile app, notifications) and task titles are lower-stakes than secrets; "private (sealed)" is one switch and turns those views off for that board. A Vault is never readable. Alternative: every board sealed, which makes the console and mobile views impossible for cloud boards.
3. **One board in the first build, room for more; several Vaults from the Vault's first phase** (§9), or several boards from the start.
4. **Cloud tier timing:** LAN first, cloud second (§12), or both together.

## 12. Build plan and who builds what

| Phase | Contents | Builder |
|---|---|---|
| S0 | The change log and state tables; hybrid clock; the adapter trait; Local collections for the Kanban (the log is written; nothing is sent) | Naki-2 |
| S1 | LAN transport (`sync.*` ops), the loop, membership, keys and the join flow; the Kanban adapter; migration and merge of per-machine boards | Naki-2 |
| S2 | The Vault adapter: sealed records, keep-both, versions, activity as Append, key epochs and re-seal. (The Vault spec's own phases VS1 to VS4, several local Vaults and the AWS Secrets Manager link among them, sit outside the layer.) | Agent2-6 |
| S3 | The cloud tier's client: the store interface, the wake, chunk upload and download; the managed service's side | Naki-2 (client), AgentY's lane (service, private spec) |
| S4 | Several collections: board switcher, per-collection agent lists, the conflict inbox, the console's collection views | Naki-2, Agent1 (UI kit), Clamk (console) |

Each phase is one or two PRs with tests; the layer is tested against an in-memory pair and trio of fake machines (partition, rejoin, clock skew, removed member, tombstone horizon) before any transport exists.

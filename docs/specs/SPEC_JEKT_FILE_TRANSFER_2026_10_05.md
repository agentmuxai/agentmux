# SPEC: Send files with a jekt, on every delivery tier

**Date:** 2026-10-05, revised 2026-10-15 against `main` 53682b5bc
**Status:** active — Phases 1 to 4 shipped (#4965, #4972, #4978, #4979, #4984); Phase 5 (sealed file keys, §7) in review
**Author:** AgentX@narko; revision by Loap@narko
**Builds on:** `SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md` and `SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md` (the attachment store and how providers receive attachments), `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md`, `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`, `SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md`, `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md`, `SPEC_WAN_JEKT_SIGNED_BY_SRV_2026_10_14.md` (signing per tier), `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` (held messages), `SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md`.

---

## 0. The ask

> can you jekt images/files to other agents? … write a spec for file transfer among agents via the jekt system. same channel -> host -> LAN -> cloud tier system

Today an agent can't. The operator wanted to pass a screenshot to an agent on another machine; the quickest workaround was transcribing it into text. It happened again on 2026-10-13: a macOS test's four screenshots stayed on the tester's machine.

## 0.1 What the 2026-10-15 revision changed, and why

The first version was written before ten days of jekt work landed. Checked against `main` 53682b5bc:

1. **The proposed signing labels were taken.** `amx-jekt-channel-v2` and `amx-jekt-lan-v2` now name the UID-bound schemes (#4577, #4579), with other fields. The argument that kept v1 and v2 material apart also relied on receivers enforcing the msgid's format, which no receiver does: msgids are uuids, chat-bridge ids or free text, depending on the source. The revision binds files through the **msgid** instead (§3.3): every scheme already signs it, so no scheme changes, and one check on receipt covers every tier.
2. **srv signs WAN jekts now** (#4902), and a host-tier request from an agent is verified by its token, not the HMAC (#4580). The host HMAC can only be checked by the instance that minted it, so a jekt forwarded to another instance on the same machine arrives as `DELIVERY=channel`. §3.3 and §4 follow this.
3. **One request instead of upload, then jekt.** The first version staged blobs with `PUT`, deduplicated with `HEAD`, and needed signed upload intents, per-sender staging quotas, a per-sender index (so `HEAD` couldn't probe the store) and a replay record. Sending the files in the same request as the jekt, after it, removes all of that: the receiver verifies the jekt's signature before it reads a single file byte, and nothing is ever staged without a verified jekt naming it (§4). Deduplication is dropped; resending a screenshot is rare and cheap.
4. **LAN files need a verified sender.** The first version both delivered files from `network-claimed` senders (§6.4) and refused LAN uploads from any peer without a published key (§4.3). The revision keeps the stricter rule: on LAN, files travel only with `TRUST=lan-verified` (§6.4).
5. **Inline images are their own phase.** The jekt delivery path carries text only (`MessageSender` is `Fn(&str, &str)`), so every provider gets the file list with paths first, which Claude and the others can open; inline blocks for persistent Claude follow (§8).
6. **Smaller facts:** `MAX_MESSAGE_LENGTH` is bytes, not characters; relative paths resolve against `AGENTMUX_AGENT_WORKDIR`; held jekts only ever originate on the receiving instance, so their files are already in its store; a sender's text can contain a fake `<attached_files>` block, which must be neutralised (§6.3); stored names are capped at 120 characters by the store's own `safe_file_name`.

## 1. What exists

- **A jekt carries one text field.** `SendMessage` takes `to` and `message` ([`crates/mcp/src/tool_schemas.rs`](../../crates/mcp/src/tool_schemas.rs)); `InjectRequest` ([`crates/common/src/api_types.rs`](../../crates/common/src/api_types.rs)) has no attachment field. Every tier truncates the body to 10,000 bytes (`MAX_MESSAGE_LENGTH`, `backend/reactive/mod.rs`), and the cloud relay client refuses more than 10,240 bytes (`MAX_RELAY_MESSAGE_BYTES`, `muxbus/relay.rs`).
- **Delivery cascade** (`deliver()`, `server/reactive.rs`): (1) same instance; (2a) same host and channel, through the per-channel registry, and (2b) same host, other channel, through the host-global shared registry, both forwarding over loopback to the peer's `/agentmux/reactive/inject` with its full auth key, and both labelled `DELIVERY=channel` on arrival; (3) LAN, peer found by mDNS/UDP discovery, plain HTTP with the peer's `lan_key`, which is broadcast in the clear; (4) cloud relay, claim before deliver, 30-minute expiry. A jekt for an agent defined on this instance but not running is held for up to 24 hours instead.
- **Signing** ([`crates/common/src/jekt_sign.rs`](../../crates/common/src/jekt_sign.rs)): the MCP signs host HMAC, LAN v1 and v2, and cross-channel v1 and v2; srv signs WAN at the origin. Every scheme signs the msgid, source, target, timestamp and message, joined with `\u{1}`. A same-instance agent is verified by its token.
- **The attachment store does the receiving half.** `get_mux_data_dir()/attachments` is content-addressed (SHA-256). It sniffs the kind and MIME from the content, makes a provider-sized send copy for images and a text version for documents, and hands files to every provider as an `<attached_files>` list of local paths, plus inline image and PDF blocks for persistent Claude (`backend/attachments/`). `POST /api/v1/attachments/upload` streams one file in and returns its `AttachmentInfo`; `incoming/` is swept after an hour, unsent files after 7 days.
- **No capability advertisement exists.** `AgentEntry` and `GET /agentmux/reactive/agent` say nothing about features; only LAN discovery and the cloud lease carry a version.

## 2. Goals and non-goals

Goals:
1. An agent sends one or more files with a jekt, to any agent it can jekt today, on any tier.
2. The receiving agent gets them exactly like files the user attached: in its attachment store, listed in the prompt, inline where its provider supports it (§8 Phase 3), visible in its pane.
3. Files are authenticated as strongly as the message text on that tier: the same signature covers them, and every byte is checked against its SHA-256.
4. The jekt message itself stays small; files never travel inside the message text.
5. Bounded: size limits per tier, a cap on concurrent incoming transfers, nothing kept from a peer unless a verified jekt names it.

Non-goals (v1):
- Sending folders or streaming large files (logs over 50 MB, videos). Use a repository or object storage and send a link.
- Confidentiality beyond what the message text has on each tier today (§7).
- Users sending files to agents; the composer already does that.
- Files from container agents (their MCP can't reach the upload route yet); the tool says so.

## 3. Sending

### 3.1 The tool

`SendMessage` gains an optional `files` parameter: a list of file paths, at most 10. A relative path resolves against `AGENTMUX_AGENT_WORKDIR` (the agent's workspace; it does not follow a `cd` in the agent's shell, so the tool description recommends absolute paths). Example:

```json
{ "to": "AgentA", "message": "Here's the shortcuts panel; note the blank space.", "files": ["C:/…/screenshots/shortcuts.png"] }
```

The MCP server reads each file (it runs as the agent, so it reads exactly what the agent can, which answers the first version's open question 1) and streams it to its own srv's `POST /api/v1/attachments/upload`, which puts it in the sender's attachment store and returns its SHA-256, size, kind and MIME. An agent forwards a file it received by passing the path it was given in its file list; the store keeps one copy per content hash, so nothing is stored twice.

Refusals happen before anything is sent: a missing or unreadable file, a directory, more than 10 files, a file over the per-file limit of the most permissive tier (§5), or a target that can't receive files (§3.4). The tool's answer names the file and the reason. The tier limit is checked again in srv once the tier is known.

### 3.2 The manifest

The jekt gains a manifest, one entry per file, in send order:

```json
[{ "sha256": "2c17…", "name": "shortcuts.png", "size": 412345 }]
```

`name` is the base name only, reduced by the store's `safe_file_name` (no path or control characters, at most 120 characters). `sha256` is always the hash of the file's **plaintext** bytes. MIME is not in the manifest: the receiver sniffs it from the content. On the cloud tier each entry also carries `key`, the file's encryption key, and `blob_id`, the relay's name for the uploaded ciphertext (§4.4).

The **manifest digest** is SHA-256 over `"amx-jekt-files-v1"` and the entries in order, each entry's fields `sha256, name, size` (and, on the cloud tier, `key, blob_id`) joined with `\u{1}`, entries joined with `\u{2}`, label and entries joined with `\u{2}`.

### 3.3 Binding the files to the signature: the msgid

A jekt with files has a msgid that ends in `.files-v1.<manifest digest in lowercase hex>`; the sender appends it to the msgid it would otherwise use. Every signing scheme on every tier already signs the msgid, so the signature now covers the file list and, through each `sha256`, every byte of every file. No scheme changes, and text-only jekts are byte-identical to today's.

The receiver runs **one check on every tier**, before reading any file byte:
- a manifest is present: the msgid must end in `.files-v1.` followed by the digest of that manifest, or the request is refused (`400`, nothing delivered);
- the msgid ends in `.files-v1.<digest>` but no manifest came: the text is delivered and its marker says the files it was sent with didn't arrive.

What the signature proves is then exactly what it proves today, per tier: `host-verified` (by token, on the origin instance), `channel-verified`, `lan-verified`, `wan-verified` or `SIG=verified`. Where a tier has no proof, the files have none either (§6.4).

Who appends the suffix: the MCP for every tier it signs (it has the upload answers), and srv for WAN, where srv chooses the msgid and signs (`wan_sign_at_origin`). srv validates the manifest and refuses a request whose suffix doesn't match before delivering or forwarding it, and checks each file's size against its own store before listing it. `MSGID=` shows the whole msgid, so the sender's `id=` and the receiver's `MSGID=` stay the same string.

### 3.4 Capability

A receiver advertises that it understands files: `jekt_features: ["files-v1"]` in its `AgentEntry` (both registries) and in its `GET /agentmux/reactive/agent` answer, and, in Phase 4, its cloud subscription. A sender only attaches files to a target that advertises `files-v1` on the tier it reaches it by; otherwise `SendMessage` fails with "AgentA's AgentMux is too old to receive files" and sends nothing, rather than delivering text that refers to files that aren't there. The same instance always qualifies.

## 4. Moving the bytes, per tier

On the host and LAN tiers the files travel **in the same request as the jekt, after it**: `POST /agentmux/reactive/inject-files`, with the body

```
u32 big-endian length of the JSON | the inject request JSON, with `files` = the manifest | file 1 bytes | file 2 bytes | …
```

each file exactly its manifest `size` long, in manifest order. The receiver:
1. reads the length and the JSON (at most 64 KB), and runs the same checks `/agentmux/reactive/inject` runs: tier, every signature, the msgid binding (§3.3), and the trust rules of §6.4. It refuses a request whose target isn't on this instance: a jekt with files is never forwarded further (no extra hops, no re-sending bytes);
2. checks the tier limits (§5) and the concurrent-transfer cap against the declared sizes, before reading on;
3. streams each file into `attachments/incoming/`, hashing as it writes, refusing a file whose bytes don't match its `sha256` or run past its `size`; a file that fails is reported, the rest still arrive;
4. commits each verified file into its store (the upload path's `commit_upload`) and delivers the jekt with its files (§6).

The answer is the same JSON `/agentmux/reactive/inject` gives, plus per-file results, so the sender's cascade and `SendMessage` report it the same way. Temporary files are removed when the request ends, whatever the outcome; `incoming/` is swept hourly as a backstop.

The jekt's own signature is what authenticates the bytes, so no separate upload intent, staging area, per-sender index or `HEAD` probe is needed. A replayed request is refused by its signature's freshness window and, for files, by the receiver remembering each accepted files jekt's msgid for that window.

### 4.1 Same instance

No copy. Sender and receiver share one attachment store; the receiver's delivery resolves the ids directly.

### 4.2 Same host, another instance (`DELIVERY=channel`)

`POST /agentmux/reactive/inject-files` on the peer's loopback URL, with its full auth key from the registry, as a jekt forward is today. The full key already allows reading that instance's whole store, so nothing more is authenticated here; the jekt's channel signature decides trust as usual.

### 4.3 LAN

The same route, added to the routes a LAN peer may call with its `lan_key` (`lan_forward_routes`). The `lan_key` proves nothing (it is broadcast), so on LAN the receiver refuses the files, before reading them, unless the jekt's LAN signature verifies against the sender's pinned key (`TRUST=lan-verified`). An unverified LAN sender's text can still go by `/agentmux/reactive/inject`; the sender's `SendMessage` says the files weren't sent and why. Transfers are plain HTTP, like LAN jekts today (§7).

### 4.4 Cloud relay

The relay can't carry large bodies and shouldn't stream them through its request path. It gains a small blob interface (the client contract is here; the relay side is built in the cloud repository):

1. `POST /reactive/blob` with the sender's account credentials and `{size, target_agent}` returns a new `blob_id` and a short-lived upload URL.
2. The sender's srv encrypts the file (below) and uploads the ciphertext.
3. srv sends the jekt as today, signed at the origin, with the manifest; each entry also has `blob_id` and `key`. The relay refuses a jekt naming a `blob_id` it doesn't hold for that sender and target.
4. The receiver claims the pending jekt as it does today, then asks `GET /reactive/blob/<blob_id>?msgid=…` for a short-lived download URL, valid only for the target's own subscription. It downloads, decrypts, checks the hash and commits the file; if a download fails it releases the claim (`/reactive/release`), so the message is retried. (The first version downloaded before claiming; today's receiver claims first, and holding the claim while downloading keeps two seats from both delivering it.)
5. Blobs live as long as the message (30 minutes plus a grace period) and are deleted on claim or expiry.

Every send encrypts each file under a fresh random key (AES-256-GCM, random nonce) and uploads it again as a new `blob_id`; there is no deduplication, so the relay never learns that two uploads are the same file. The key and `blob_id` are part of the manifest entry, so the msgid binding covers them. The receiver decrypts (the GCM tag rejects altered ciphertext), then checks the plaintext against `sha256`. From Phase 5 the key is sealed to the receiving install (§7.1), so neither the storage nor the relay can read the file; to an install that predates sealing it travels as plain base64, which protects against the storage being read on its own, not against the relay.

### 4.5 Held messages

Only a jekt that started on the receiving instance is ever held (its target defined there but not running, or not signed in), so its files are already in that store. The held row gains a `files_json` column with the manifest; replay passes it on. Unsent files are kept 7 days, longer than a hold's 24 hours; a file missing at replay is reported as not delivered, like any missing file.

## 5. Limits

| | Same instance / host | LAN | Cloud |
|---|---|---|---|
| Per file | 100 MB | 50 MB | 25 MB |
| Per jekt | 10 files, 200 MB | 10 files, 100 MB | 10 files, 50 MB |
| Incoming transfers at once, per receiver | 4, 1 GB declared in total | (shared with host) | n/a |

Settings keys follow the attachment store's (`jekt:files:maxfilemb`, `jekt:files:maxjektmb`, per tier). The receiver's normal attachment limits (total store size, image dimensions, text extraction) still apply; a file the store refuses is reported back as not delivered.

## 6. Receiving

### 6.1 Checks before delivery

The receiver delivers a jekt's files only when:
1. its trust allows files (§6.4);
2. the msgid binds the manifest (§3.3);
3. each file's bytes match its `sha256` and `size`, and the store accepts it.

A file that fails 3 is left out and the marker says which and why; the others are delivered. The sender's `SendMessage` answer lists what was delivered.

### 6.2 What the agent sees

The marker gains `FILES=<n>` (the number delivered), and the file list goes inside the jekt, after the message and before the closing divider, outside the 10,000-byte message limit:

```
[JEKT:FROM=agentx TO=agenta TIER=coord DELIVERY=channel TRUST=channel-verified FILES=1 MSGID=… PRIORITY=normal TS=…]
…
Here's the shortcuts panel; note the blank space.
<attached_files>
agentx sent 1 image with this message. Open it from this path with your file or image viewing tool when you need it.
1. shortcuts.png — C:\…\attachments\named\2c17…\shortcuts.png
</attached_files>
…
[/JEKT]
```

The list comes from the same code as user attachments (`backend/attachments/prompt.rs`: send copy for images, text version for documents), with a header naming the sender. The pane shows the files as attachment chips on the jekt bubble, read back from that list as user messages' chips are.

### 6.3 Names and content

Names are reduced by `safe_file_name` and stored under `named/<sha256>/`. Kind and MIME are sniffed from the content; nothing from the sender decides them. Nothing received is ever executed or opened automatically; executables and archives are stored and listed like any other file.

A sender's message text can't produce a file list: `<attached_files>` and `</attached_files>` in it are neutralised like marker text is, and the pane reads chips from a jekt only when its marker has `FILES=`.

### 6.4 Trust

Files follow the jekt's trust and tier rules:

| Sender | Files |
|---|---|
| **Verified** on its tier (`host-verified`, `channel-verified`, `lan-verified`, `wan-verified` with an `approved` or `new` instance, `SIG=verified`) | delivered; inline for persistent Claude from Phase 3 |
| **No proof available**, same machine or cloud (`self-declared`, WAN `network-claimed`) | delivered and listed by path, never inlined, so a file from an unproven sender isn't put in front of the model unasked |
| **No proof available**, LAN (`network-claimed`) | refused before any byte is read (§4.3) |
| **Active verification failure** (`TRUST=unverified`, `SIG=invalid`, a LAN, channel or WAN signature that was present and failed, a revoked instance) | withheld: none delivered, the marker says they were withheld |

The credential/destructive keyword scan that can force `TIER=sensitive` also runs over file names.

## 7. Confidentiality

Files are as private as the message text on the same tier, no more:
- same instance and same host: never leave the machine;
- LAN: plain HTTP, readable by anyone who can see the LAN traffic, exactly like LAN jekt text today;
- cloud: TLS to the relay and encrypted at rest with a per-file key. From Phase 5 that key is sealed to the receiving install (§7.1), so the relay can't read the file either. To an install that predates sealing the key travels as plain base64, and the file is readable by the relay, like jekt text.

### 7.1 Sealed file keys (Phase 5)

The text of a cloud jekt stays readable by the relay; a file sent with it no longer is, when the receiving install can open a sealed key.

**The receiver's key.** Each agent's sealing key is an X25519 key derived from its WAN seed: HKDF-SHA256 with salt `amx-jekt-files-seal-key-v1` and info `x25519`. It is not the WAN signing key itself (the curve library advises against using one key for both), nothing new is stored, and the same seed always gives the same key.

**The offer.** When an install claims an agent's lease it adds `seal: {instance, key_fp, key, sig}`: its WAN instance id, the fingerprint of the agent's WAN key, the X25519 public key, and the agent's WAN key's Ed25519 signature over `amx-jekt-files-seal-key-v1`, the agent id (lowercase), the instance id, the fingerprint and the key, joined with `\u{1}`. The relay checks each field's shape and passes the offer on in the lease holder's answer; it is trusted for nothing.

**The check.** A sender whose target's holder offers a key fetches the agent's record from the account's key directory (the instance, the lease's channel and the fingerprint name it), checks its certificate chain and that its install isn't revoked, then checks the offer's signature with the record's key. The relay can't put its own key in an offer without the install's instance key, which never leaves that install. A holder that offers a key gets sealed keys or nothing: if the check fails or the directory can't be reached, `SendMessage` says why and nothing is sent. A holder that offers none (an older install, or an agent with no WAN key) gets plain keys, as in Phase 4.

**The seal.** For each file, after its blob is uploaded: a fresh ephemeral X25519 key; the shared secret with the receiver's key (an all-zero result is refused); HKDF-SHA256 with salt `amx-jekt-files-seal-v1` over it, with info the ephemeral key, the receiver's key, the file's `sha256` and its `blob_id`, giving an AES-256-GCM key; that key encrypts the file key under a zero nonce (each wrapping key is used once). The manifest's `key` becomes `x25519.<agent key fingerprint>.<base64url of the ephemeral key, the wrapped key and the tag>`, 158 characters, and the msgid binding covers it like a plain key.

**Opening.** The receiver opens a sealed key with the claimed agent's WAN seed before downloading the blob. A key sealed to another fingerprint (the lease moved to another install between send and claim) or one that doesn't open is left out like any file that fails, and the others arrive.

**What it doesn't stop.** A cloud that actively adds an install to the account's key directory and takes the agent's lease could be offered files, the same residual WAN verification accepts for a `new` instance (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.6); and a relay that strips the offer from a lease gets plain keys, as an older install would. Sealing removes the passive exposure: the relay's storage, logs and operators no longer hold anything that opens the file.

## 8. Plan

| Phase | Work |
|---|---|
| 1 | Same instance. `SendMessage.files` (MCP uploads to its srv); manifest, msgid binding and the receive-side check; same-instance delivery; held jekts keep their manifest; `FILES=`, the sender-named `<attached_files>` list, neutralising fake lists and quoting sender-controlled names; keyword scan over names; chips on the jekt bubble |
| 2 | Another instance on this machine, and LAN: `POST /agentmux/reactive/inject-files` (in `lan_forward_routes`), `files-v1` in registry entries and the LAN agent answer, files only from `lan-verified` senders on LAN, a LAN files jekt delivered once, four transfers at once per receiver |
| 3 | Inline images and PDFs for a running persistent Claude from verified senders: `MessageSender` carries Claude content blocks; the files step inlines only for a verified sender and a target that is running or starting, within the session's inline budget, so the list's "shown above" is always true; a target that has to be started gets the list alone |
| 4 | Cloud: the relay's blob interface (cloud repository); the sender encrypts each file (AES-256-GCM, fresh key and nonce), uploads it, and rebinds the msgid to the cloud manifest (its own digest label, keys and blob ids included) so srv's WAN signature covers them; the receiver fetches after the claim, checks a verified sender's binding, decrypts, checks the hash, and releases the claim when the relay can't be reached. A blob is fetchable within its jekt's window, capped at 23 hours after upload |
| 5 | Sealed file keys (§7.1): the receiving install offers an X25519 key, derived from the agent's WAN seed and signed by its WAN key, in the agent's lease; the relay passes it on and accepts sealed keys in manifests (cloud repository); the sender checks the offer against the key directory and seals each file key to it; the receiver opens it before downloading |

## 9. Tests

1. A text-only jekt's request and signatures are byte-identical to today's on every tier.
2. A jekt with files verifies on each tier; changing one byte of a file, its name, its size, the file order, the message text, or (cloud) a file's key or `blob_id` fails: the msgid no longer binds the manifest, or a file's hash doesn't match.
3. A manifest whose digest isn't the msgid's suffix is refused before any file byte is read; a bound msgid with no manifest delivers the text and says the files didn't arrive.
4. A file whose bytes don't match its hash, or run past its size, is left out and reported; the others arrive. No temporary file outlives its request.
5. Sending to a target without `files-v1` fails without sending anything.
6. A LAN jekt with files from a sender that isn't `lan-verified` is refused before its files are read; one replayed within its window is refused.
7. Over the per-file, per-jekt or concurrent-transfer limits: refused before reading the files.
8. A held jekt keeps its files until delivery or expiry.
9. Cloud: a failed download releases the message; a download URL works only for the target's subscription; blobs disappear after claim and after expiry.
10. Trust, per §6.4's table, including an active failure withholding every file. A credential keyword in a file name forces `TIER=sensitive`.
11. Names with path separators, control characters or a misleading extension are sanitised; MIME comes from the content.
12. `<attached_files>` in a sender's message text produces no file list for the agent and no chips in the pane.
13. Sealed keys: a sealed key opens only with its agent's WAN seed, for its own file and blob; a changed byte, another agent's key, another file's hash or blob id fails; an offer is accepted only with the directory record that proves it, so a key the relay substitutes, another agent's offer or a broken record is refused; the sealing key is not the signing key.

## 10. Open questions

1. Should the keyword scan also run over the extracted text of documents (the first version said yes)? Code and logs say "token" or "password" constantly, so it would flag a large share of ordinary files, and the scan would have to wait for text extraction before delivering. Phase 1 scans names only.
2. Cloud quota per account (total bytes in flight), and whether files count toward any plan limits.
3. Whether a received file should also be copied into the receiving agent's working directory (as `attachments.copy-to-workdir` does for container panes), or stay in the store only.

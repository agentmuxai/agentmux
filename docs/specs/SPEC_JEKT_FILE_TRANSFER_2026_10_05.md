# SPEC: Send files with a jekt, on every delivery tier

**Date:** 2026-10-05
**Status:** proposed
**Author:** AgentX@narko
**Builds on:** `SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md` and `SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md` (the attachment store and how providers receive attachments), `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md`, `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`, `SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md`, `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` (signing per tier), `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` (held messages), `SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md`.

---

## 0. The ask

> can you jekt images/files to other agents? … write a spec for file transfer among agents via the jekt system. same channel -> host -> LAN -> cloud tier system

Today an agent can't. The operator wanted to pass a screenshot to an agent on another machine; the quickest workaround was transcribing it into text.

## 1. What exists

- **A jekt carries one text field.** `SendMessage` takes `to` and `message` (`crates/mcp/src/tool_schemas.rs`); `InjectRequest` has no attachment field. Every tier truncates the body to 10,000 characters (`MAX_MESSAGE_LENGTH`, `backend/reactive/mod.rs`), and the cloud relay refuses more than 10 KB (`MAX_RELAY_MESSAGE_BYTES`, `muxbus/relay.rs`).
- **Delivery cascade** (`deliver()`, `server/reactive.rs`): (1) same instance, in-memory registry; (2a) same host and channel, (2b) same host, other channel, through the host-global shared registry, forwarding over loopback HTTP to the peer's `/agentmux/reactive/inject`; (3) LAN, peer found by mDNS/UDP discovery, plain HTTP with the peer's `lan_key`; (4) cloud relay, POST to the relay, receiver pulls over its subscriber connection, claim before deliver, 30-minute expiry.
- **Signing covers the message text** on every tier: host HMAC, cross-channel Ed25519, LAN Ed25519, WAN instance-certified Ed25519 (`crates/common/src/jekt_sign.rs`), fields joined with `\u{1}`.
- **An attachment store already does the receiving half.** `get_mux_data_dir()/attachments` is content-addressed (SHA-256), makes a provider-sized send copy for images and a text version for documents, and hands files to every provider as an `<attached_files>` list of local paths (plus inline image and PDF blocks for persistent Claude), with limits, retention and cleanup (`backend/attachments/`). The image attachment spec lists "attachments over WAN/jekt" as out of scope; this spec is that follow-up.
- **No file transfer between instances exists** (no blob, upload or presign path besides the attachment upload from the local frontend).

## 2. Goals and non-goals

Goals:
1. An agent sends one or more files with a jekt, to any agent it can jekt today, on any tier.
2. The receiving agent gets them exactly like files the user attached: in its attachment store, listed in the prompt, inline where its provider supports it, visible in its pane.
3. Files are authenticated as strongly as the message text on that tier: the same signature covers them, and every byte is checked against its SHA-256.
4. The jekt message itself stays small; files never travel inside the message text.
5. Bounded: size limits per tier, quotas per receiver, short-lived staging, no unbounded disk use from unauthenticated peers.

Non-goals (v1):
- Sending folders or streaming large files (logs over 50 MB, videos). Use a repository or object storage and send a link.
- Confidentiality beyond what the message text has on each tier today (§7).
- Users sending files to agents; the composer already does that.

## 3. Sending

### 3.1 The tool

`SendMessage` gains an optional `files` parameter: a list of file paths (absolute, or relative to the agent's working directory), at most 10. Example:

```json
{ "to": "AgentA", "message": "Here's the shortcuts panel; note the blank space.", "files": ["screenshots/shortcuts.png"] }
```

The MCP server resolves each path (it runs as the agent, so it can read what the agent can read), then calls the server to **ingest** each file into the sender's attachment store (the existing ingest path: classification, size checks). It gets back an attachment id (the SHA-256), the stored name, size and MIME type. An agent can also forward a file it received by passing its attachment id (`"attachment:<sha256>"`).

Refusals happen before anything is sent: a missing or unreadable file, a directory, more than 10 files, or a file over the per-file limit of the tier that will be used (§5). The tool's answer names the file and the reason.

### 3.2 The manifest

The jekt gains a manifest, one entry per file, in send order:

```json
[{ "sha256": "2c17…", "name": "shortcuts.png", "size": 412345, "mime": "image/png" }]
```

`name` is the base name only (no path), at most 128 characters after sanitising (§6.3). `sha256` is always the hash of the file's **plaintext** bytes. On the cloud tier each entry also carries `key`, the file's encryption key (§4.4).

### 3.3 Signing

Every tier's signed material gains one field, the **manifest digest**: SHA-256 over the canonical manifest (entries in order; each entry's fields `sha256, name, size, mime` and, on the cloud tier, `key`, joined with `\u{1}`; entries joined with `\u{2}`). It is appended after `message` in each scheme, and every v2 scheme starts with its own version label, so v1 and v2 material can never be confused: the message is free text and may contain `\u{1}`, so without a label a v1 message ending in `\u{1}<digest>` could reproduce a v2 input. v1 material can't start with a v2 label either, because its first field is the msgid, whose fixed format the receiver already enforces.

- host HMAC: `"amx-jekt-host-v2", …, message, files_digest`
- cross-channel: `"amx-jekt-channel-v2", …, message, files_digest`
- LAN: `…, message, files_digest`, under the label `"amx-jekt-lan-v2"`
- WAN: `"amx-jekt-wan-v2", …, message, files_digest`

A jekt without files keeps the v1 material unchanged, so nothing changes for text-only traffic. With files, the signature binds the message, the file list, and through the digests every byte of every file: the receiver checks each blob's SHA-256 against the signed manifest, so a blob swapped in transit (or in a store) is rejected.

### 3.4 Capability

A receiver advertises that it understands files: `"jekt_features": ["files-v1"]` in its agent registry entry (`AgentEntry`), in the LAN `GET /agentmux/reactive/agent` answer, and in its cloud subscription. A sender only attaches files to a target that advertises `files-v1` on the chosen tier; otherwise `SendMessage` fails with "AgentA's AgentMux is too old to receive files (needs 0.6x)" and sends nothing, rather than delivering the text without the files it refers to.

## 4. Moving the bytes, per tier

One rule on every tier: **the bytes move first, in the same direction as the jekt, then the jekt names them.** Pushing (rather than letting the receiver pull) needs no reachability beyond what the jekt itself needs, which matters on LAN, where the firewall may allow only one direction.

Every receiver has a **staging area** for blobs that have arrived but aren't yet claimed by a delivered jekt: `attachments/incoming/jekt/` (the store already has `incoming/`). A staged blob does nothing on its own. It is promoted into the store only when a jekt whose signature verifies names it, and it is deleted after one hour if none does.

### 4.1 Same instance

No copy. Sender and receiver share one attachment store; the jekt carries the manifest and the receiver's delivery path resolves the ids directly.

### 4.2 Same host (same or other channel)

Two instances on one machine have separate data folders and stores.

- The sender asks `HEAD /agentmux/jekt/blob/<sha256>` on the receiver (loopback). `200` means this receiver already holds that blob **from this same sender** (an earlier send of the same screenshot), so it is skipped. The answer is `404` for anything else, including blobs the receiver holds from other senders or from its own user, so the endpoint can't be used to probe what files a receiver has. The receiver keeps a per-sender index of the blobs it received from each sender for this.
- Otherwise `PUT /agentmux/jekt/blob/<sha256>` streams the file. The receiver hashes as it writes and refuses on a mismatch, a size over the tier limit, or a full quota (§5).
- Then the usual forward to `/agentmux/reactive/inject`, with the manifest.

Authentication is the same as the inject on that hop (the peer's auth key from the registry entry, loopback only), plus the signed upload intent of §4.3.

### 4.3 LAN

The same `HEAD`/`PUT` endpoints, on the peer's LAN address, added to the routes a LAN peer may call with its `lan_key`. The LAN key is broadcast in the clear, so it proves nothing about who is uploading. Every `HEAD` and `PUT` therefore carries a **signed upload intent**: the sender's LAN Ed25519 signature (the same per-agent key as its LAN jekts) over `"amx-jekt-blob-v1", source, target, sha256, size, ts`, in the headers `X-Jekt-Source` and `X-Jekt-Blob-Sig`. The receiver verifies it against the claimed sender's published LAN public key (as it does for a LAN jekt), refuses an unsigned or failing request, and charges the upload to that verified sender. So the per-sender quota can't be dodged by inventing names, and a peer that has no agent with a published key can't upload at all. The same intent is required on the host tier, signed with the host key or the cross-channel key.

On top of the per-sender quota there is a **global staging cap** per receiver (§5), and the one-hour expiry; nothing staged is delivered without a signed jekt naming it. Transfers are plain HTTP, like LAN jekts today (§7).

### 4.4 Cloud relay

The relay can't carry large bodies, and shouldn't stream them through its own request path. It gains a small blob interface (the client contract is defined here; the relay's storage is implemented in the cloud repository):

1. `POST /reactive/blob` with the sender's account credentials and `{sha256, size, target_agent}` returns a short-lived upload URL (or `exists: true` if the relay already holds that blob for that target).
2. The sender uploads the file to that URL.
3. The sender sends the jekt as today, now carrying the manifest; the relay refuses a jekt whose manifest names a blob it doesn't hold.
4. When the receiver pulls the pending jekt, it asks `GET /reactive/blob/<sha256>?msgid=…` and gets a short-lived download URL, valid only for the target's own subscription. It downloads, checks the hash, ingests, and only then claims the message, so a failed download leaves the message to be retried.
5. Blobs live as long as the message (30 minutes plus a grace period) and are deleted on claim or expiry.

Every blob is encrypted before upload (AES-256-GCM, a random key and nonce per file). The key travels in the manifest entry (`"key"`), which is part of the canonical entry and so covered by the signed manifest digest (§3.3). The receiver decrypts (the GCM tag rejects altered ciphertext), then checks the plaintext against `sha256`, the same check as on every other tier. That protects against the storage being read on its own (a misconfigured bucket, a backup), not against the relay itself, which already sees message text (§7).

### 4.5 Held messages

A jekt that is held because the target isn't running (`SPEC_DURABLE_JEKT_DELIVERY`) holds its files too: they are promoted into the receiver's store when the jekt is held, and are marked in use until it is delivered or its 24 hours run out.

## 5. Limits

| | Same instance / host | LAN | Cloud |
|---|---|---|---|
| Per file | 100 MB | 50 MB | 25 MB |
| Per jekt | 10 files, 200 MB | 10 files, 100 MB | 10 files, 50 MB |
| Receiver staging, per verified sender | 500 MB | 200 MB | n/a (relay side) |
| Receiver staging, all senders together | 1 GB | 1 GB | n/a |

Settings keys follow the attachment store's (`jekt:files:maxfilemb`, …). The receiver's normal attachment limits (total store size, image dimensions, text extraction) still apply on ingest; a file the store would refuse is reported back as not delivered.

## 6. Receiving

### 6.1 Checks before delivery

The receiver delivers a jekt with files only when:
1. it is not an active verification failure (§6.4): a jekt with a verified signature, or one whose tier has no proof to offer (`self-declared`, `network-claimed`), qualifies; one whose signature was present and failed does not;
2. every manifest entry's blob is staged (or already in the store) and its SHA-256 matches;
3. each file passes the store's ingest checks.

If a blob is missing or wrong, the jekt is still delivered with the files it does have, and its marker says which file failed and why. The sender's `SendMessage` answer reports what was delivered, as it does for text today.

### 6.2 What the agent sees

The marker gains `FILES=<n>`:

```
[JEKT:FROM=agentx TO=agenta TIER=coord DELIVERY=wan TRUST=wan-verified … FILES=1 MSGID=… TS=…]
Here's the shortcuts panel; note the blank space.
<attached_files>
1. shortcuts.png — C:\…\attachments\named\2c17…\shortcuts.png (image/png, 403 KB, from agentx)
</attached_files>
[/JEKT]
```

Files go through the same pipeline as user attachments (`backend/attachments/prompt.rs`): the send copy for images, a text version for documents, the `<attached_files>` list for every provider. The pane shows them as attachment chips on the jekt bubble, labelled with the sender.

### 6.3 Names and content

Names are reduced to a base name, control and path characters removed, and made unique in `named/`. MIME type is sniffed from the content, not taken from the manifest. Nothing received is ever executed or opened automatically; executables and archives are stored and listed like any other file.

### 6.4 Trust

Files follow the jekt's trust and tier rules. One rule per kind of sender:

| Sender | Files |
|---|---|
| **Verified** on its tier (`host-verified`, `channel-verified`, `lan-verified`, `wan-verified`, `SIG=verified`) | delivered; inline for persistent Claude |
| **No proof available** (`self-declared`, `network-claimed`) | delivered and listed by path, never inlined, so a file from an unproven sender isn't put in front of the model unasked |
| **Active verification failure** (`TRUST=unverified`, `SIG=invalid`, a LAN or WAN signature that was present and failed, a revoked instance) | withheld: none delivered, the marker says they were withheld |

In addition, the credential/destructive keyword scan that can force `TIER=sensitive` also runs over file names and the extracted text of documents.

## 7. Confidentiality

Files are as private as the message text on the same tier, no more:
- same instance and same host: never leave the machine;
- LAN: plain HTTP, readable by anyone who can see the LAN traffic, exactly like LAN jekt text today;
- cloud: TLS to the relay, encrypted at rest with a per-file key, but readable by the relay, like jekt text.

A later version can seal each file key to the receiver's public key (an X25519 key published next to its signing key), which would make cloud files unreadable to the relay. The manifest's `key` field is where that change would land.

## 8. Plan

| Phase | Work |
|---|---|
| 1 | `SendMessage.files`; ingest on send; manifest and v2 signatures (with v1 unchanged for text); `files-v1` capability; same-instance and same-host delivery (`HEAD`/`PUT` blob, staging, promotion); marker and `<attached_files>` rendering; pane chips |
| 2 | LAN: blob routes for LAN peers, per-sender staging quota |
| 3 | Cloud: relay blob interface (cloud repository), encrypted upload/download, claim after download |
| 4 | Sealed file keys (§7) |

## 9. Tests

1. A text-only jekt's signatures are byte-identical to today's on every tier; a v1 message containing `\u{1}` followed by a digest never verifies as v2 (every v2 scheme is labelled).
2. A jekt with files verifies on each tier; changing one byte of a file, its name, the file order, the message text, or (cloud) a file's key fails verification.
3. A blob whose content doesn't match its hash is refused at upload and at promotion.
4. A staged blob not named by a signed jekt within an hour is deleted; a verified sender over its staging quota is refused, and so is any upload once the global staging cap is reached.
4a. LAN and host uploads without a valid signed upload intent are refused; inventing a sender name doesn't get past the per-sender quota.
4b. `HEAD` answers 200 only for blobs received from the same sender; a blob the receiver holds from anyone else answers 404.
5. Sending to a target without `files-v1` fails without sending anything.
6. The same file sent twice moves once (`HEAD` answers 200).
7. A held jekt keeps its files until delivery or expiry.
8. Cloud: a failed download leaves the message pending; a download URL works only for the target's subscription; blobs disappear after claim and after expiry.
9. Trust, per §6.4's table: verified senders get files inline; `self-declared` and `network-claimed` senders get files listed but not inlined; an active verification failure withholds every file. A credential keyword in a document's text forces `TIER=sensitive`.
10. Names with path separators, control characters or a misleading extension are sanitised; MIME comes from the content.

## 10. Open questions

1. Should an agent be able to send a file from outside its working directory? This spec allows anything the agent can read, matching what it could paste as text; a stricter version limits it to the working directory.
2. Cloud quota per account (total bytes in flight), and whether files count toward any plan limits.
3. Whether a received file should also be copied into the receiving agent's working directory (as `attachments.copy-to-workdir` does for container panes), or stay in the store only.

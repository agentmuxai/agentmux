# Sharing widgets: signed packages, widgets in agent bundles, a catalog

**Status:** active — phase W6 of `SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md` §13. W6a (signed packages, §2) in this PR; W6b and W6c not started.
**Date:** 2026-10-10
**Builds on:** `SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md` (packages §5, the content hash §8.2, approval §8.3)

## 1. Goal

A widget written by one person can reach another without losing what W1–W5 guarantee: nothing runs until the user approves it in AgentMux's own UI, and the prompt says exactly what it may do. W6 adds three things, each usable on its own:

1. **Signed packages:** the prompt can say who published a widget, and an update from a different publisher can't pass as the same widget's.
2. **Widgets in agent bundles:** a bundle can carry the widgets its agents use; installing the bundle offers them, never approves them.
3. **A catalog:** a list of sandboxed widgets the user can browse and install from inside AgentMux.

## 2. Signed packages

### 2.1 The signature file

A signed package has `widget.sig` at its root, next to `widget.json`:

```json
{
  "sigVersion": 1,
  "algorithm": "ed25519",
  "publicKey": "<base64, 32 bytes>",
  "signature": "<base64, 64 bytes>"
}
```

The signature is Ed25519 over the bytes `agentmux-widget-sig-v1\0<id>\0<version>\0<content hash>`. The content hash is §8.2's, computed over every file **except `widget.sig`** (a file can't sign itself), so signing a package doesn't change what was approved and an approval doesn't depend on whether a signature is attached. `widget.sig` is never served by the widget files route (it isn't one of the approved files).

### 2.2 Publisher keys

A key is shown by its fingerprint: the first 10 bytes of SHA-256 of the public key, base32 (RFC 4648, no padding), in groups of four: `K7Q2-MZ4D-PX3A-9TWE`. Someone who wants their widgets recognised publishes their fingerprint where users can check it (their homepage, the catalog).

The manifest's publisher (the part of `id` before the dot) is free-form (decision 6 of the widget spec's §3). A signature ties it to a key **on this machine**, trust-on-first-use, the way SSH remembers a host:

- When the user approves a signed package whose publisher this instance hasn't seen, srv pins `publisher → public key` (in its data directory, next to the approvals).
- A later package with that publisher is checked against the pin.

### 2.3 What srv reports, and what the prompt says

`widgets.list` gains `signature` for each package:

| `signature.state` | When | The prompt says |
|---|---|---|
| `signed` | It verifies, and the publisher is pinned to this key | "Signed by acme · `K7Q2-MZ4D-PX3A-9TWE` (the same key as your other acme widgets)" |
| `signed_new` | It verifies; the publisher isn't pinned yet | "Signed by acme · `K7Q2-MZ4D-PX3A-9TWE`. This is the first acme widget here: check the key with its author if it matters." |
| `unsigned` | No `widget.sig`; the publisher isn't pinned | "Not signed: AgentMux can't tell who made it." |
| `key_changed` | It verifies, but the publisher is pinned to another key; or it isn't signed and the publisher is pinned | A warning: "**Not signed by the key that signed your other acme widgets.** It may not be from the same author." The button reads **Install anyway**. |
| `invalid` | `widget.sig` is there but doesn't parse or doesn't verify | The package is `invalid` (§8.1): "its signature doesn't match its files". It can't be installed. |

- The approval record keeps the signing key. A package whose files are unchanged but whose signature now names another key, or was removed, is `changed` and asks again with the full prompt.
- An update (`changed`) whose signature state is `key_changed` always gets the full prompt, never the short "was updated" one.
- Approving a `key_changed` package does **not** move the pin. Settings → Widgets lists the pinned publishers, with **Forget key**, for an author who really changed keys. Forgetting goes through the host-only route an approval uses (decision `forget_key`): with only srv's auth key, which every agent has, a pin can't be removed.
- `WidgetInstall` returns the signature state, so an agent can tell the user what the prompt will say.

### 2.4 Signing a package

The npm package gains a command line, with no dependencies beyond Node's own Ed25519:

```bash
npx -p @agentmuxai/widget-sdk agentmux-widget keygen ~/.agentmux-widget-key.json   # once; keep it private
npx -p @agentmuxai/widget-sdk agentmux-widget sign ./acme.pr-dashboard --key ~/.agentmux-widget-key.json
npx -p @agentmuxai/widget-sdk agentmux-widget verify ./acme.pr-dashboard            # prints the fingerprint
```

`sign` computes §8.2's hash exactly as srv does, so a package that verifies with `verify` verifies in AgentMux. The fixture `sdk/widget-sdk/fixtures/acme.fixture/` (stored byte-exact, `-text`) is signed with a fixed test seed; srv's `widget_signature.rs` and the SDK's `widget-sign.test.ts` both check its hash, signature and fingerprint. Signing is the last step: any edit afterwards breaks the signature (`invalid`) until the package is signed again.

### 2.5 What a signature doesn't mean

It names a key, not a person, and it says nothing about what the code does. The permissions in the prompt are still the whole of what a sandboxed widget can do; a trusted widget's prompt still says it has full access. The prompt words it that way.

## 3. Widgets in agent bundles

### 3.1 Today

A bundle is an Agent Bundle Format zip (`.abf`, v0.3, `backend/bundle_export.rs`, `bundle_import.rs`): instructions, per-provider instructions, context files and skills, all text. Import is preview, then commit, bound by a content digest (`bundle.import.preview`, `bundle.import.commit`, the three import modals in `frontend/app/view/bundle/components/`). Nothing in a bundle runs code, and nothing is signed; non-UTF-8 entries are skipped.

### 3.2 The format: ABF v0.4 adds `widgets/`

```
<slug>/bundle.json                  "components": { …, "widgets": [{ "id", "version", "hash" }] }
<slug>/widgets/<id>/widget.json     a whole package, as in ~/.agentmux/widgets/<id>/
<slug>/widgets/<id>/widget.sig      optional (§2)
<slug>/widgets/<id>/…               its files, binary included
```

- Entries under `widgets/` are read as bytes (an icon, a font, a wasm file), the one place in a bundle that isn't text. The widget limits apply (spec §5.1: 50 MB, 2,000 files, no links), inside the bundle's own zip limits, which rise for a bundle with widgets to the widget limit plus the rest.
- `components.widgets[].hash` is the package's content hash (§8.2). Import refuses a widget whose files don't hash to it, so the preview the user saw is what is installed.
- **Sandboxed widgets only.** A bundle with a trusted widget is refused at preview ("bundles can carry sandboxed widgets only"). A trusted widget is full access to AgentMux; it shouldn't arrive as a side effect of importing an agent's instructions. A user who wants one installs it from its folder, with that prompt.
- An importer older than v0.4 ignores `widgets/` (it already skips what it doesn't know); the rest of the bundle imports as before.

### 3.3 Import

- The preview modal gains a **Widgets** section: each widget's name, version, signature line (§2.3) and permissions in the words of spec §6.4, with a checkbox, on by default. The preview says plainly: "Each widget asks for your approval before it runs."
- Commit copies each chosen widget into `~/.agentmux/widgets/<id>/` (asking before it replaces an installed version, as **Install from folder** does), then opens the normal approval prompt for each. **A bundle never approves a widget**: they arrive `needs_approval` (or `changed`), exactly as if installed from a folder, and declining one leaves the rest of the bundle imported.
- An installed, approved widget at the same hash stays approved; nothing asks again.
- `bundle.import_for_agent` (agents importing a bundle through the App API) does the same: the widgets wait for the user's approval, which an agent can't give.

### 3.4 Export

- The bundle editor (Memory → Bundles) gains **Widgets**: pick installed sandboxed widgets to include. Export writes their approved files (never a newer, unapproved copy on disk), with `widget.sig` if the package has one.
- `bundle.export` takes `widgets: [id]`.

### 3.5 What the agent is told

Nothing new: an agent that uses a widget already learns its view from `WidgetList`. A bundle's instructions can mention its widgets by view (`ext:acme.board/main`); `OpenWidget` opens one once the user has approved it.

## 4. A catalog

### 4.1 What it is

A public list of sandboxed widgets, browsable in Settings → Widgets → **Browse**, installed with the same prompt as any widget. The catalog is a convenience and a second check, never an approval: a widget from it is `needs_approval` like any other.

### 4.2 Where it lives

**Decided (repo owner, 2026-10-10): A, a new public repo `agentmuxai/widgets`.**

| Option | How | For | Against |
|---|---|---|---|
| **A. A public repo `agentmuxai/widgets`** (recommended) | One folder per widget (`widgets/<id>/`, its source), submitted by PR; CI validates and builds `index.json` and one `.zip` per version, published with GitHub Pages | Contributions and review are PRs; CI checks every rule before a human looks; history of every version; independent of app and docs releases | One more public repo to own |
| B. The docs site | `agentmux-docs` serves `index.json` and the zips next to the docs | No new repo | Widget submissions mixed with docs PRs; a catalog change waits for a docs deploy |
| C. The app repo | `docs/examples/widgets/` is the catalog | Nothing new | Only AgentMux's own samples; every catalog change is an app PR |

### 4.3 The index

```json
{
  "catalogVersion": 1,
  "generated": "2026-10-20T12:00:00Z",
  "widgets": [{
    "id": "acme.pr-dashboard", "name": "PR dashboard", "version": "1.2.0",
    "description": "…", "author": "Acme", "homepage": "https://…", "icon": "code-pull-request",
    "permissions": ["net:https://api.github.com"],
    "hash": "<content hash>", "publisherKey": "<base64>",
    "zip": "https://…/acme.pr-dashboard-1.2.0.zip", "zipSha256": "<hex>"
  }]
}
```

`index.json.sig` is an Ed25519 signature over the index's exact bytes by the catalog key. AgentMux pins the catalog's public key in its source (like muxreview's message key), so a changed index or a different server can't add a widget.

### 4.4 What CI checks before a widget is listed

- The manifest validates (spec §5.3) with the same rules as srv (the check runs a published validator built from srv's code, so they can't drift).
- `kind` is `sandboxed`; trusted widgets are never listed.
- The package is signed (§2), and its key is the one the publisher registered in the repo's `publishers.json` on their first submission. A different key for a known publisher fails CI.
- The limits (spec §5.1), no remote code (no `http(s)://` in `<script src>` or `import`, which the CSP would block anyway), and permissions a reviewer has read: a new `net:` origin or `agents:send` needs a maintainer's approving review.

### 4.5 Installing from it

srv fetches the index (from the catalog URL in settings, default the official one), checks its signature, and lists it. **Install** has srv download the zip, check `zipSha256`, the package hash and the author's signature against the index, copy it into `~/.agentmux/widgets/<id>/`, and open the approval prompt, which adds "From the AgentMux catalog" and, when the publisher key matches the catalog's, "Publisher verified by the catalog". An installed widget whose catalog entry has a newer version shows **Update** in Settings; an update is a new approval (spec §8.3).

## 5. Phases

| Phase | Builds | Done when |
|---|---|---|
| **W6a: signed packages** | `widget.sig`, the hash rule, verification and the publisher pins in srv; `signature` in `widgets.list` and `WidgetInstall`; the prompt lines and the `key_changed` warning; pinned publishers in Settings; `keygen`/`sign`/`verify` in the npm package, with a shared hash fixture | a signed sample installs with its fingerprint shown; a package re-signed with another key warns; an edited signed package is invalid (tested) |
| **W6b: widgets in bundles** | ABF v0.4 `widgets/`; binary entries for it; preview, commit, export; sandboxed only | a bundle exported with a widget imports on another instance and the widget asks for approval (tested) |
| **W6c: the catalog** | the catalog repo (or B/C), its CI, the signed index; Browse, Install and Update in Settings | the samples are in the catalog and install from Browse |

## 6. What this doesn't do

- No automatic updates: an update is always the user's approval.
- No paid widgets, ratings or reviews.
- No trusted widgets from a bundle or the catalog.

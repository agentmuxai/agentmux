# Sharing widgets: signed packages, widgets in agent bundles, a catalog

**Status:** active — phase W6 of `SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md` §13. W6a (signed packages, §2) in PR #4645; W6b (widgets in bundles, §3) in PR #4650; W6c (the catalog, §4): the `agentmuxai/widgets` repository and its signed index are live (https://agentmuxai.github.io/widgets/); AgentMux's side in PR #NNNN.
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

- The answer carries the fingerprint the prompt showed ("" for unsigned), relayed by the host with the id and hash. `widget.sig` isn't in the hash, so srv checks the signature on disk against it and refuses the approval if another key signs the package now ("the widget's signature changed since you were asked"): a signature swapped between the prompt and the click can't be recorded or pinned.
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

### 3.2 The format: a `widgets` component

```
<slug>/bundle.json                  "components": { …, "widgets": [{ "id", "version", "hash" }] }
<slug>/widgets/<id>/widget.json     a whole package, as in ~/.agentmux/widgets/<id>/
<slug>/widgets/<id>/widget.sig      optional (§2)
<slug>/widgets/<id>/…               its files, binary included
```

- The manifest keeps ABF v0.3's `$schema`: `widgets` is one more optional component, and an importer that doesn't know it imports the rest of the bundle as before (it skips the entries; the non-text ones with a warning).
- Entries under `widgets/` are read as bytes (`backend/bundle_widgets.rs`), the one place in a bundle that isn't text, and never reach the text importer. Each widget is held to the widget limits (spec §5.1: 50 MB, 2,000 files, no links, no path outside its folder), and a bundle to at most 20 widgets and 100 MB of widget files in all, decompressed (each entry is read at most one byte past what's left, so a high compression ratio can't force more). The archive itself is at most 100 MB and 10,000 entries, as for any bundle.
- `components.widgets[].hash` is the package's content hash (§8.2). A widget whose files don't hash to it, or that `bundle.json` doesn't list, isn't offered, so what the preview shows is what installs.
- **Sandboxed widgets only.** A trusted widget in a bundle is shown as "bundles can carry sandboxed widgets only" and not offered. A trusted widget is full access to AgentMux; it shouldn't arrive as a side effect of importing an agent's instructions. A user who wants one installs it from its folder, with that prompt.
- Only a zip carries widgets: the `files[]` input of `bundle.import.preview` is text.

### 3.3 Import

- `bundle.import.preview` returns `widgets`: each widget's name, version, author, kind, permissions, hash, signature (§2.3), the version installed here if any, and why it can't be imported, if it can't.
- The preview modal gains a **Widgets** section: each widget's name and version, its signature line and its permissions in the words of spec §6.4, with a checkbox, on by default. It says plainly: "Each widget asks for your approval before it runs." A widget already installed and approved at the same files and key is shown as such and not offered (one installed but still waiting is offered, and asks again); one that would replace an installed version says so.
- `bundle.import.commit` takes `include_widgets: [id]`. After the bundle is written, srv reads the archive again, holds it to the digest the user previewed, copies each chosen widget into `~/.agentmux/widgets/<id>/` (replacing an installed version), and opens the normal approval prompt for each, as "The bundle "<name>" wants to install <widget>". **A bundle never approves a widget**: they arrive `needs_approval` (or `changed`), exactly as if installed from a folder, and declining one leaves the rest of the bundle imported. The commit answers each widget's outcome: `waiting`, `unchanged` or `failed` with why.
- An agent importing a bundle through the App API gets the same: the widgets wait for the user's approval, which an agent can't give. (`bundle.import_for_agent` doesn't carry widgets yet.)

### 3.4 Export

- `bundle.export` takes `widgets: [id]` (with `format: "zip"`): installed, approved, sandboxed widgets to include. Export writes their **approved** files, each hashed again against the approval (never a newer, unapproved copy on disk), with `widget.sig` when it still names the approved signer, and lists them in `components.widgets`.
- AgentMux's UI has no bundle export button today; agents export through the App API. A widget picker belongs with that button when it comes.

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

`index.json.sig` is an Ed25519 signature over the index's exact bytes by the catalog key, base64. AgentMux pins the catalog's public key in its source (`backend/widget_catalog.rs` `CATALOG_KEYS`, a list so a key can be rotated), so a changed index or a different server can't add a widget. Every `zip` must be on the index's own origin, and srv follows no redirects. The private half of the catalog key lives only in the catalog repository's Actions secret `CATALOG_SIGNING_SEED`.

### 4.4 What CI checks before a widget is listed

The catalog repository's `scripts/build.mjs`, on every pull request and on `main` (no dependencies, Node's own Ed25519):

- The manifest: version 1, an id that is its folder's name, semver, a name, at least one pane whose entry file is in the package.
- `kind` is `sandboxed`; trusted widgets are never listed.
- It is signed (§2), the signature matches its files (the same hash rule as srv and the SDK), and its key is the one the publisher registered in the repository's `publishers.json`. A different key for a known publisher fails.
- The limits (spec §5.1), no links, and no `<script src>` or `<link href>` to another origin (the CSP would block them anyway).
- A maintainer reads every submission before it is merged; a new `net:` origin or `agents:send` gets a closer look.

AgentMux checks again on install, as it does for any widget (§4.5): the catalog's checks are its own gate, never the last one.

### 4.5 Installing from it

- `widgets.catalog` (srv) fetches `index.json` and its signature from `https://agentmuxai.github.io/widgets/`, checks the signature, and lists each widget with its publisher's fingerprint and what is installed here (`current` when the installed files are the catalog's).
- Settings → Widgets → **Browse the catalog** lists them: name, version, author, the fingerprint, and the permissions in the prompt's words, with **Install**, or **Update to <version>** for an installed older one.
- `widgets.catalog.install` fetches and checks the index again, downloads the widget's zip, and checks the zip's SHA-256, the package's content hash, its kind, and its author's signature against the index (the key must be the entry's `publisherKey`). Only then does it copy the widget in, as `needs_approval`.
- The approval prompt opens right away in Settings, with one more line when the package is exactly what the catalog lists: "From the AgentMux catalog: its files and its publisher's key are the ones the catalog lists." An update is a new approval (spec §8.3).

## 5. Phases

| Phase | Builds | Done when |
|---|---|---|
| **W6a: signed packages** | `widget.sig`, the hash rule, verification and the publisher pins in srv; `signature` in `widgets.list` and `WidgetInstall`; the prompt lines and the `key_changed` warning; pinned publishers in Settings; `keygen`/`sign`/`verify` in the npm package, with a shared hash fixture | a signed sample installs with its fingerprint shown; a package re-signed with another key warns; an edited signed package is invalid (tested) |
| **W6b: widgets in bundles** | the `widgets` component and `widgets/<id>/` entries, read as bytes; preview, commit and export; sandboxed only | a bundle exported with a widget imports on another instance and the widget asks for approval (tested) |
| **W6c: the catalog** | the `agentmuxai/widgets` repository, its CI and Pages site, the signed index, the catalog key pinned in AgentMux; `widgets.catalog` and `widgets.catalog.install`; Browse, Install and Update in Settings | the samples are in the catalog and install from Browse |

## 6. What this doesn't do

- No automatic updates: an update is always the user's approval.
- No paid widgets, ratings or reviews.
- No trusted widgets from a bundle or the catalog.

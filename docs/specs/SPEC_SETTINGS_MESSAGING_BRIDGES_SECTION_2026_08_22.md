# SPEC — Settings: new "Integrations" section (Discord / Telegram / Slack / WhatsApp bridges)

**Date:** 2026-08-22
**Type:** Feature (design proposal — not yet implemented)
**Status:** Draft
**Scope:** New `frontend/app/view/settings/sections/integrations-section.tsx` (+
registration in `settings-view.tsx`/`settings-model.ts`), reusing the
`MaskedKeyField` primitive introduced by `SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md`.
No backend changes — all four bridges are already fully wired and read their
config from `settings.json` via the generic `setconfig` merge path.

## Why this needs its own spec

Candidate #3 from `docs/specs/SPEC_SETTINGS_AUDIT_GOOD_PICKINGS_2026_08_19.md`
(tracked in #2671), explicitly deferred to a follow-up spec rather than
designed in full in that pass: *"the most 'product-shaped' gap in the whole
audit — arguably bigger in scope than the Recording section (4 separate
integrations vs. 1)... Left as a follow-up spec of its own given the scope."*
This is that follow-up. Candidates #2 and #4 from the same audit (agent
watchdog thresholds, drag-and-drop settings) and candidate #1 (Recording) have
already shipped (#2748, #2744, #2751) — this closes out the audit's
remaining item.

## Problem

Four messaging bridges — Discord, Telegram, Slack, WhatsApp Cloud API — are
real, fully wired server-side integrations (`agentmux-srv/src/bootstrap.rs`,
`agentmux-srv/src/server/messaging_handlers.rs`, ~24 dedicated fields across
`agentmux-srv/src/backend/wconfig/types.rs:295-437`), each with its own
enable toggle, bot token(s), and per-bridge routing config. **None of this
has any Settings UI, and none of it is even present in `schema/settings.json`**
(confirmed by grep — zero `messaging:*` schema entries exist today, unlike
every other settings family). The only way to configure any of these four
integrations is hand-writing raw JSON into `settings.json`, including
multiple bot tokens and app secrets in plaintext.

## Current state (grounded in `types.rs`)

### Discord (`messaging:discord:*`)
| Key | Type | Purpose |
|---|---|---|
| `enabled` | bool | Master toggle — connects to the Discord Gateway at startup when true |
| `token` | string, secret | Bot token (discord.com/developers → Bot → Token) |
| `channel` | string | Channel ID filter + default send target |
| `target` | string, optional | Agent ID receiving inbound messages via the reactive bus (absent = logged, not forwarded) |
| `guild` | string, optional | Guild ID for guild-scoped slash commands (Phase 2) |

### Telegram (`messaging:telegram:*`)
| Key | Type | Purpose |
|---|---|---|
| `enabled` | bool | Master toggle — starts long-polling `getUpdates` at startup |
| `token` | string, secret | Bot token from @BotFather |
| `allowed_chats` | string (comma-separated) | Chat-ID allowlist; anything else silently dropped |
| `default_chat` | string, optional | Default outbound chat ID |
| `target` | string, optional | Agent ID receiving inbound messages |

### Slack (`messaging:slack:*`)
| Key | Type | Purpose |
|---|---|---|
| `enabled` | bool | Master toggle — opens a Socket Mode connection at startup |
| `bot_token` | string, secret | `xoxb-...` — Web API calls (`chat.postMessage`) |
| `app_token` | string, secret | `xapp-...` — Socket Mode only (`connections:write` scope) |
| `channel` | string | Channel ID filter + default send target |
| `target` | string, optional | Agent ID receiving inbound messages |

### WhatsApp Cloud API (`messaging:whatsapp:*`) — largest, per `SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07.md`
| Key | Type | Purpose |
|---|---|---|
| `enabled` | bool | Master toggle — outbound sender starts, `/webhook/whatsapp` routes resolve |
| `phone_number_id` | string | Meta App Dashboard → WhatsApp → API Setup |
| `access_token` | string, secret | System User permanent access token |
| `app_secret` | string, secret | Validates `X-Hub-Signature-256` on inbound webhooks |
| `webhook_verify_token` | string, secret | GET handshake verify token (user-chosen, must match Meta dashboard) |
| `target` | string, optional | Agent ID receiving inbound messages |
| `fallback_template` | string, optional | Template name for sends outside the 24h customer-service window |
| `fallback_template_lang` | string, optional | Template language (BCP-47), default `en_US` |
| `tunnel_domain` | string | Cosmetic only — prints the full callback URL in startup logs; v1 does not manage a tunnel |

Cloud API only, no Baileys/unofficial mode (deliberate v1 scope cut, see the
WhatsApp spec §2.1) — so there is exactly one "mode" per bridge, unlike
`voice:engine`'s multi-engine picker.

## Design

### 1. Section placement

New rail entry **"Integrations"** (not "Messaging" — reserves that word for a
future in-app chat feature if one is ever built; "Integrations" also reads
naturally as a home for any *other* future external-service bridge, not just
chat platforms). Position: after "Recording", before "Advanced" — both are
"external capability, currently invisible" sections in the same vein as the
audit's ranking. New file
`frontend/app/view/settings/sections/integrations-section.tsx`.

### 2. Layout — one collapsible sub-block per bridge

```
Integrations
├── Discord         [toggle: enabled]
│     ▸ (expands when enabled)
│       Bot token         [MaskedKeyField]
│       Channel ID        [text]
│       Target agent      [text — see §4 below on validation]
│       Guild ID          [text, optional]
├── Telegram        [toggle: enabled]
│     ▸ Bot token         [MaskedKeyField]
│       Allowed chat IDs  [text, comma-separated, placeholder "123,456"]
│       Default chat ID   [text, optional]
│       Target agent      [text]
├── Slack           [toggle: enabled]
│     ▸ Bot token (xoxb-) [MaskedKeyField]
│       App token (xapp-) [MaskedKeyField]
│       Channel ID        [text]
│       Target agent      [text]
└── WhatsApp        [toggle: enabled]
      ▸ Phone number ID       [text]
        Access token          [MaskedKeyField]
        App secret            [MaskedKeyField]
        Webhook verify token  [MaskedKeyField]
        Target agent          [text]
        Fallback template     [text, optional]
        Fallback template lang [text, optional, default "en_US"]
        Tunnel domain         [text, optional — cosmetic, see note below]
```

Each bridge's `enabled` toggle gates its own sub-block via `<Show>`, exactly
like `sounds-section.tsx`'s master-toggle-gates-sub-rows pattern (already
established convention — `notify:sounds:enabled` gating the rest of that
section). No accordion/collapse animation needed for v1 — reuse the plain
`Show` gate, not a new collapsible-panel primitive.

### 3. Masked fields — reuse `MaskedKeyField` as-is

Every secret field (`token`, `bot_token`, `app_token`, `access_token`,
`app_secret`, `webhook_verify_token`) uses the `MaskedKeyField` primitive
`settings-controls.tsx` already has (introduced in
`SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md` §2, generalized on
purpose for this exact reuse — see that spec's Open Question 1). No new
component needed. Each field gets its own short egress note reusing that
primitive's existing shape, e.g.:

- Discord token: *"Used only to connect to Discord's Gateway from this
  machine; never sent anywhere else."*
- Slack tokens: *"`xoxb-` calls Slack's Web API; `xapp-` opens a Socket Mode
  connection — both stay on this machine."*
- WhatsApp `access_token`/`app_secret`/`webhook_verify_token`: *"Used only for
  this machine's calls to the Graph API and to validate inbound webhooks."*

Non-secret identifier fields (`channel`, `phone_number_id`, `guild`,
`allowed_chats`, `default_chat`, `fallback_template*`, `tunnel_domain`) are
plain `setting-text` inputs, same pattern as every other section.

### 4. "Target agent" fields — validate against real agent IDs

All four bridges have a `target` field (agent ID receiving inbound messages).
Today this is a bare string with no validation — a typo silently means
"messages logged but never forwarded" with no feedback. Recommend a
`<select>` populated from the existing agent-list RPC (whatever
`AgentApi`/`FleetApi` call already backs the agent picker elsewhere in
Settings/Armory — reuse, don't re-fetch a duplicate list) instead of a free
text field, with a "custom ID" escape hatch for an agent not yet running.
This is a real UX improvement over the raw string every bridge has today, not
scope-creep — the audit's own framing values "closing a real gap," and a
silently-swallowed typo here is exactly that.

### 5. WhatsApp's `tunnel_domain` — copy, not a control

Per the field's own doc comment, this is cosmetic-only (prints a callback URL
in server logs; v1 does not supervise a tunnel). Render it as a plain text
input like the others, but the row's description text should say so
explicitly: *"Used only to print your webhook URL in the server log at
startup — you're responsible for the tunnel (Cloudflare Tunnel, ngrok, etc.)
and registering the URL with Meta yourself."* Don't imply this field starts
or manages anything.

### 6. Bridge-specific validation notes (surfaced as description text, not new backend calls)

- **Telegram `allowed_chats`**: parsed as `Vec<i64>` at startup — the row's
  description should say "comma-separated numeric chat IDs" so a
  non-numeric entry doesn't silently fail to parse. No live validation RPC in
  v1 (unlike the Recording section's `voice.checkPath` — there's no cheap
  server-side check analogous to "does this file exist" for a chat ID; it can
  only be confirmed by receiving a real message).
- **Slack**: both tokens are required together for Socket Mode to actually
  connect; the section should note this adjacency (*"Both tokens are required
  — Socket Mode needs the app-level token to open the connection and the bot
  token to post messages"*) rather than let a user save just one and wonder
  why nothing happens.

## Non-goals

- **No live "test connection" flow** for any bridge in v1 (unlike Recording's
  "test your microphone") — each bridge's actual startup/connect sequence
  already logs success/failure server-side (`bootstrap.rs`); duplicating a
  connection attempt from a Settings save action risks double-connecting a
  Gateway/Socket Mode session. A future "Reconnect now" button that restarts
  just that bridge is a reasonable follow-up, not designed here.
- **No OAuth flow for Slack app installation** — this section only stores the
  two tokens a user obtains manually from api.slack.com; it does not walk a
  user through creating a Slack app or installing it to a workspace.
- **No tunnel management for WhatsApp** — confirmed out of scope by the
  underlying WhatsApp spec (§2.1) already; this section doesn't change that.
- **No new "test/validate this token" endpoint** for any bridge, unlike
  Recording's `voice.checkPath`. A bad Discord/Slack/Telegram token today only
  surfaces as a connection failure in server logs at next startup — adding a
  lightweight "does this token work" probe per bridge is a reasonable
  follow-up but quadruples this spec's backend surface (4 new endpoints, one
  per bridge's own API shape) for a v1 that's already large. Flagged as the
  natural next increment once this section ships.

## Open questions

1. **"Integrations" vs. "Messaging" as the rail label** — leaning
   "Integrations" per §1's reasoning above; confirm before implementing since
   renaming later means finding and updating every reference (same class of
   churn `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` had to do for
   "preset" → "bundle").
2. **Agent-picker reuse (§4)** — which existing RPC/component already lists
   live agents for a picker (check Armory's agent-binding UI and the
   `AgentPicker.tsx` component referenced in recent commits) before building
   a new one.
3. **Restart-on-change semantics** — do these bridges hot-reload when their
   settings change (like `network:lan_discovery`'s `apply()` being
   idempotent-callable), or do they only read config at process startup
   (`bootstrap.rs`)? If the latter, the section needs a "restart required"
   note similar to `term:disablewebgl`'s existing pattern — confirm against
   `bootstrap.rs`/`messaging_handlers.rs` before implementing, since this
   materially changes what the UI should tell the user after a save.

## References

- `docs/specs/SPEC_SETTINGS_AUDIT_GOOD_PICKINGS_2026_08_19.md` — candidate #3,
  the audit this follows up on.
- `docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md` —
  `MaskedKeyField`'s origin spec; §2 and Open Question 1 anticipate this
  exact reuse.
- `docs/specs/SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07.md` — WhatsApp
  bridge's own design, including the Cloud-API-only v1 scope decision this
  spec inherits.
- `agentmux-srv/src/backend/wconfig/types.rs:295-437` — the full field set
  this spec's UI surfaces (Discord/Telegram/Slack/WhatsApp settings structs).
- `agentmux-srv/src/bootstrap.rs`, `agentmux-srv/src/server/messaging_handlers.rs`
  — bridge startup/wiring, relevant to Open Question 3.
- `frontend/app/view/settings/sections/sounds-section.tsx` — the
  toggle-gates-sub-rows pattern this section's per-bridge collapsing reuses.
- `frontend/app/view/settings/sections/recording-section.tsx`,
  `frontend/app/view/settings/settings-controls.tsx` — `MaskedKeyField` and
  general section conventions this spec follows.

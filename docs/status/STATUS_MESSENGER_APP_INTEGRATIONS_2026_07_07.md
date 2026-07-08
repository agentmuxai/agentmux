# Status — Messenger App Integrations (Discord / Slack / Telegram / WhatsApp / Teams)

**Date:** 2026-07-07
**Type:** Living status doc (snapshot of a dormant program)
**Program:** Embed the 5 highest-reach messaging apps as real, agent-connected panes inside
AgentMux, per `SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24.md`. No dedicated GitHub issue or
discussion exists for this feature — it has been tracked entirely through specs + merged PRs.
The adjacent, broader "OpenClaw as a universal agent-to-human channel router" vision (issue
[#102](https://github.com/agentmuxai/agentmux/issues/102)) is a separate, longer-term,
not-yet-built path and should not be confused with this one.

---

## 1. Where we are (one-glance)

| Workstream | State |
|---|---|
| Pane layer (all 5 apps as CEF webview panes) | ✅ Shipped — merged in #1763 (2026-06-24), icons fixed in #1777 (2026-06-25) |
| Bridge framework (`MessagingBridge` trait, shared scaffold) | ✅ Merged (#1763) — `agentmux-srv/src/messaging/mod.rs` |
| **Discord bridge (Phase 1)** | ✅ **Implemented and functional** — Gateway WS + REST send, opt-in via `settings.json`, `POST /api/messaging/discord/send` |
| Slack bridge (Phase 2) | ⬜ Not started — pane only, widget description says "(bridge Phase 2)" |
| Telegram bridge (Phase 2) | ⬜ Not started — pane only, widget description says "(bridge Phase 2)" |
| WhatsApp bridge (Phase 3) | ⬜ Not started — pane only, widget description says "(bridge Phase 3)"; also policy-blocked, see §4 |
| Teams bridge (Phase 3) | ⬜ Not started — pane only, widget description says "(bridge Phase 3)"; "Implement Last" per spec, enterprise-only |
| Bridge health surfaced in Warden widget | ⬜ Not started — Warden's "Internet" section is still a stub |
| Settings UI for messaging bridges | ⬜ Not started — Discord bridge is config-file-only (no in-app UI) |
| Pluggable/community bridge system (Signal, Matrix, iMessage, WeChat, …) | ⬜ Not spec'd — referenced but the API design doc doesn't exist yet |
| Program activity | 🔴 **Dormant ~11 days.** Last touch: 2026-07-01 (#1876, an unrelated security-tier fix to already-shipped Discord code — not new platform work). No open PR or active branch for any of the remaining 4 bridges. |

---

## 2. What's actually implemented today

**All 5 apps are, right now, plain webview panes** — a CEF browser widget pointed at the
platform's real web app (`discord.com/app`, `app.slack.com`, `web.telegram.org`,
`web.whatsapp.com`, `teams.microsoft.com`), address bar hidden. Registered as built-in
("Tier 1") widgets in `agentmux-srv/src/config/widgets.json:121-195`. This is the user's
correct read — no custom chat UI exists for any of them; the messaging app *is* the UI.

**Discord is the one exception**, and it's a real, working exception, not a stub:
`agentmux-srv/src/messaging/discord/` (~700 LOC across `gateway.rs`, `rest.rs`, `types.rs`,
`mod.rs`) implements an actual Discord Gateway WebSocket client (HELLO → IDENTIFY/RESUME →
heartbeat → `MESSAGE_CREATE`) plus REST send (`POST /channels/{id}/messages`). It starts only
if `messaging.discord.enabled` + `messaging.discord.token` are set
(`agentmux-srv/src/main.rs:731-750`), and is reachable via `POST /api/messaging/discord/send`.
An agent can already read/send Discord messages that appear natively in the pane a human is
looking at — the actual point of "refining the integration" beyond a plain webview.

Notably, the implementation diverged from the original plan: the POC spec recommended the
`serenity`/`twilight-rs` crates, but the shipped code uses raw `tokio-tungstenite` + `reqwest`
directly — a pragmatic zero-new-dependency choice that worked.

Slack, Telegram, WhatsApp, and Teams have **no bridge code** — no subdirectory under
`messaging/`, none of the platform-specific libraries the plan calls for
(`teloxide`, `@slack/socket-mode`, Baileys) are in `Cargo.lock`. This isn't hidden: each
widget's own description string in `widgets.json` says "(bridge Phase 2)" / "(bridge Phase 3)"
today, visible in the app.

## 3. The plan for "refining the integration"

Three specs, all dated 2026-06-24, all still `Status: Draft` (none promoted to
approved/implemented despite Discord already shipping in `main`):

- **`SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24.md`** — the master plan. Two-layer
  architecture (pane + background bridge), a `MessagingBridge` trait every platform
  implements, a `TunnelManager` for platforms needing a public webhook URL (WhatsApp Cloud,
  Teams), and a 5-phase roadmap: **Telegram → Discord → Slack → WhatsApp → Teams** (note
  Discord actually shipped first, out of the planned order). §7 explicitly frames all of this
  as a stopgap: once OpenClaw ships, a single `OpenClawBridge` could implement the same trait
  and retire the per-platform bridges.
- **`SPEC_MESSAGING_INTEGRATION_DISCORD_POC_2026_06_24.md`** — the Discord POC spec, executed
  (with the crate substitution noted above).
- **`ANALYSIS_PLUGIN_WIDGET_MESSAGING_INTEGRATION_2026_06_24.md`** — sizing/architecture.
  Proposes a Tier 1 (built-in) / Tier 2 (external-UI) / Tier 3 (background-daemon) widget
  model to formalize where bridges belong; all 5 raw bridges would add well under 1MB to the
  binary. Flags a future Tier 4 plugin system for community-contributed bridges as
  **"❌ Not spec'd."**

Concretely, "refining the integration" means: background bridges (agent posts/reads through
the real UI, not a custom re-implementation), rich per-platform output (Discord Embeds,
Telegram inline keyboards, Slack Block Kit, Teams Adaptive Cards), bridge connection-health
surfaced in the Warden widget, OS-keychain credential storage + webhook signature validation,
and eventually a pluggable system for community-contributed platforms. Native SDK embedding,
notification badges, a unified inbox, and deep linking are **not** part of the stated plan —
don't assume those are coming.

## 4. Per-platform status + known blockers

| App | Bridge status | Notable blocker/limitation from the spec |
|---|---|---|
| Discord | ✅ Live | Heartbeat-ghost risk (bot silently stops receiving events); needs proactive reconnect on OS wake-from-sleep. No external health monitor exists yet. |
| Slack | Not started | Socket Mode connections refresh ~hourly and can silently stop delivering after days — needs proactive reconnect + a 5-min-no-event force-reconnect heuristic. |
| Telegram | Not started | Lowest-friction of the remaining four (long-polling, no tunnel needed) — a reasonable next pick despite not being first historically. |
| WhatsApp | Not started | Most complex. Official Cloud API needs Meta Business verification (2-10 days) + a dedicated number + a public HTTPS webhook (tunnel required). **Meta's Oct-2025 policy bans using WhatsApp as a delivery channel for a general-purpose AI assistant distributed to others** — spec says a personal single-user bot is "gray area" and to frame AgentMux's use as a personal productivity bridge, not a chatbot product. Unofficial (Baileys) path carries a 15-30% account-ban risk for bots that message new contacts proactively; spec requires a persistent user-acknowledged warning banner before enabling it. |
| Teams | Not started | Explicitly "Implement Last." Requires Azure Bot Service + an Azure subscription + Entra ID tenant + admin-gated app sideloading. **Not usable at all for personal/non-enterprise users.** |

## 5. Tracking

No dedicated tracking issue or discussion thread exists for this feature by name. Work has
been tracked purely through the three specs above and two merged PRs:

- PR [#1763](https://github.com/agentmuxai/agentmux/pull/1763) — bridge framework + Discord
  Gateway integration (merged 2026-06-24).
- PR [#1777](https://github.com/agentmuxai/agentmux/pull/1777) — FontAwesome brand-icon fix
  for the 5 widgets, which originally rendered blank (merged 2026-06-25).

Issue [#102](https://github.com/agentmuxai/agentmux/issues/102) ("OpenClaw — Open-Source Agent
Deployment Platform", open) is adjacent but distinct: it envisions an agent reaching a human
through *any* channel (Discord/Telegram/Slack/Signal/iMessage/WhatsApp) via ACP, not
platform-specific bridges. A pinned maintainer comment on that issue already flags it needs
re-scoping to `SPEC_OPENCLAW_AGENT_2026_05_17.md` or closing as superseded — it should **not**
be treated as the tracking issue for the webview/bridge work described here.

## 6. Gaps / recommended next step

- **No settings UI.** The only way to enable the Discord bridge is hand-editing
  `settings.json` — there's no in-app affordance to paste a token, which likely limits who's
  even using the one bridge that exists.
- **No health visibility.** `BridgeHealth` is designed in the spec but not wired into the
  Warden widget's "Internet" section (itself still a stub), so a silently-dead Discord bridge
  (the heartbeat-ghost failure mode called out above) would currently go unnoticed.
- **Docs drift**: `docs/specs/pane-types.md` still lists 9 pane types while `widgets.json`
  registers 17 — Discord/Slack/Telegram/WhatsApp/Teams are among the ones missing from that
  doc (independently flagged in `docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md:256`).
- **Program has stalled at 1 of 5 bridges** with no open work. If resuming, Telegram is the
  cheapest next platform (no tunnel/webhook infra, unlike WhatsApp/Teams) and would validate
  the shared `MessagingBridge` trait against a second, different protocol shape
  (long-polling vs. Discord's WebSocket Gateway) before investing further.
- Given OpenClaw is explicitly named in the plan as the eventual replacement for
  per-platform bridges, it may be worth deciding whether to keep hand-rolling Phases 2-5 or
  wait for OpenClaw's channel-routing to mature — that's a real fork in the road the specs
  themselves flag but don't resolve.

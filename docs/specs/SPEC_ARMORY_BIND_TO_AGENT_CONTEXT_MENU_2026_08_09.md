# SPEC — Armory "Bind to Agent" context menu on account rows

**Date:** 2026-08-09
**Status:** Proposed (user-requested direction; supersedes the failure-row
surface of `SPEC_ACCOUNT_ADOPTION_PIGGYBACK_LOGIN_2026_08_09.md` as the
primary UX — see §6 for how the two relate)
**Trigger:** A user with **three** anthropic/claude accounts in the Armory
has no way to assign them to agents from the place where the accounts are
visible. Assignment today only happens agent-side (launch modal picker,
per-agent Identity tab), which means answering "which agent uses which of
my three accounts?" requires visiting every agent. Binding is a
management task; the Armory is the management surface.

---

## 1. Feature

Right-click an account row in the Armory Accounts tab (`AccountRow`,
`identity-accounts-tab.tsx:101` — currently has **no** context menu) →
native `ContextMenuModel` menu:

```
Bind to Agent            ▸   AgentA        ● running   ✓ (this account)
                             Agent1        ● running     (bound: work-claude)
                             Mzop                        (no account bound)
                             ────────────────────────
                             AgentY (codex — incompatible)   [hidden, see §3]
─────────────────
Copy account ID
```

- **"Bind to Agent" ▸ submenu** lists the channel's user-owned agent
  definitions. Clicking one binds this account to that agent.
- **Per-row annotations** (all fields the native menu already supports —
  `submenu`/`checked`/`sublabel` in `ContextMenuItem`, custom.d.ts:329):
  - `checked` — agent is already bound to *this* account.
  - `sublabel` — the currently-bound account's name when it's a
    *different* account ("bound: work-claude"), or "no account bound".
    This is what makes three-account disambiguation legible: the submenu
    doubles as a live binding overview.
  - Running marker — agents with an open pane (via
    `getOpenDefinitionMap()`, agent-pane-state-store.ts:390) sort first
    and get a running indicator; non-running agents are still bindable
    (pre-provisioning before launch is a feature, not an error).
- **"Copy account ID"** — freebie while the menu exists; the Armory rows
  are one of the copy-affordance gaps already catalogued in
  `REPORT_CONTEXT_MENU_GAP_AUDIT_2026_08_07.md` §4.

## 2. Bind action

`LinkAgentIdentityCommand { agent_id, account_id, provider: account.provider }`
— the existing upsert (`ON CONFLICT(agent_id, provider) DO UPDATE`), so
binding to an agent that already has an account for this provider
**replaces** that link. Because it replaces:

- When the clicked agent's sublabel shows a *different* bound account, the
  click is a rebind — perform it directly (the sublabel already disclosed
  what it replaces; a confirm dialog on an easily re-doable, non-destructive
  upsert is noise). The old account row is untouched — only the link moves.
- After a successful bind, the account list re-renders via the live cache
  (`identityaccounts:changed` → #2474) and the per-agent panels via their
  existing links subscriptions — no reload.

**Running-agent live apply:** if the target agent has an open pane
(`getOpenDefinitionMap()` gives the blockId), mirror the `cmd:env`
config-dir refresh `useAgentControllerStatus.useGlobalLogin()` already
performs after linking (SetMetaCommand on the block's `cmd:env` with the
provider's `authConfigDirEnvVar` → the account's `OAuthConfigDir` dir), so
the new binding takes effect on the next turn without a restart — the
same no-restart semantics the existing recovery flows promise. Without
this step a stale static `cmd:env` override could shadow the new link at
the next spawn.

## 3. Candidate agents (menu contents)

- User-owned definitions only (`is_seeded = 0`), current channel (both
  accounts and agent definitions are channel-local — cross-channel is out
  of scope, same as the companion spec).
- **CLI-OAuth accounts** (claude/codex/gemini/openclaw — oauth-class,
  `secret_ref.backend == "oauth_config_dir"`): only agents whose
  `resolve_provider_alias(def.provider)` matches the account's
  canonicalized provider. A claude account bound to a codex agent injects
  a pointless `CLAUDE_CONFIG_DIR` and confuses the binding overview —
  filter them out (hidden, not greyed: the menu is an assignment tool,
  not a diagnostic).
  - Canonicalize BOTH sides — the alias-mismatch bug class from codex P1
    on PR #2377 applies verbatim.
- **Service accounts** (github/aws/etc., api-key class): all agents are
  candidates — these links inject env vars usable by any provider's CLI.
- Empty candidate set → "Bind to Agent" renders disabled with a sublabel
  ("no compatible agents in this channel") rather than disappearing —
  discoverability over minimalism, matching the disabled-Copy precedent
  in the generic pane menu.

## 4. Out of scope

- The AgentMux Cloud row (muxbus singleton — not an IdentityAccount, no
  link semantics) and the brand gallery tiles get no menu. Accounts-list
  rows only.
- Unbind ("remove link") from this menu — deferred; the per-agent
  Identity tab already owns link removal, and a destructive action jammed
  into a bind-flavored submenu invites misclicks. Revisit if requested.
- Multi-select / bind-to-many in one gesture.
- Cross-channel and cross-machine binding (see companion spec §3).

## 5. Test plan

- Unit (menu builder, pure function → item list): provider filtering with
  aliases both ways, checked/sublabel correctness for
  bound-here/bound-elsewhere/unbound, running-first sort, disabled empty
  state, service-account provider passthrough.
- Unit (bind action): link RPC with canonical provider; `cmd:env` refresh
  fired only when the pane is open; no refresh for closed agents.
- Component test: right-click on `AccountRow` opens the menu
  (`AgentPicker.test.tsx`'s existing right-click → ContextMenuModel test
  pattern is the template).
- Live check: the motivating scenario — 3 claude accounts, right-click
  each, verify the submenu shows correct current bindings, rebind one
  agent between two accounts, confirm next turn uses the new account
  (spawn log's `injected CLAUDE_CONFIG_DIR … account=<id>` line).

## 6. Relationship to SPEC_ACCOUNT_ADOPTION_PIGGYBACK_LOGIN

Same underlying mechanism (link upsert + optional live `cmd:env` apply),
two entry points for two mindsets:

- **This spec (primary):** "I'm looking at my accounts, let me assign
  them" — proactive management from the Armory.
- **Companion's Surface B** (Identity-tab "Link existing account…"):
  reactive repair from a broken agent's side — still worth having since a
  gate-refused agent's failure message points the user at that tab.
- **Companion's Surface A** (failure-row picker) is superseded by this
  direction: the failure row keeps its existing actions, and its message
  already routes users to the Armory, where this menu now closes the loop.

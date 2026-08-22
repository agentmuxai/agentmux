# Status: The OAuth-class Credential-Isolation Spawn Gate Appears to Never Fire in Production (2026-08-20)

**Status: RESOLVED — the headline claim in this doc's own title was wrong. See §8
(2026-08-21 update).** The gate does fire and does work correctly; the "empty
identity_id" WARN that started this investigation is a red herring (binding
resolution keys off `definition_id`, not `identity_id` — see §8). The real,
still-open question this investigation surfaces is narrower and different in
kind: whether the specific account AgentY (and Lark) are bound to —
**"Claude (personal)," whose `secret_ref` points at the operator's own real,
global `~/.claude`** — was bound to these agents intentionally. Tracked
separately, see §8.

**Trigger:** Operator closed this agent's (`AgentY`) pane on AgentMux v0.55.15 and reopened
it on v0.55.18, expecting conversation continuity. None occurred — investigating why led
to a much larger finding than the original complaint. Full forensic trail:
`docs/retro/retro-agenty-global-claude-home-leak-2026-08-20.md`. This doc is the
action-oriented summary for anyone picking this up next.

Sections 1-7 below are kept as-written (the investigation as it actually happened,
including the wrong turn) rather than silently rewritten — §8 is the correction.

---

## 1. The headline finding

`agentmux-srv/src/identity/resolver/inject.rs`'s spawn-time credential gate
(`inject_identity_env_with_broker` → `gate_oauth_failure`) is supposed to unconditionally
block any spawn of an oauth-class-provider agent (claude/codex/gemini/copilot/openclaw)
that has no bound account — a deliberate hardening (commit `860fb0b6a`, 2026-07-23, "single-
point enforcement... `use_ambient_login` no longer has any effect") of the exact hole
`docs/retro/retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md` documented:
silent fallback to the user's personal, global `~/.claude` login.

**By every static read of the code, this agent's own pane (`identity_id=""`, zero rows in
`db_agent_identity_links`, provider `claude`) should be blocked on every single spawn.** It
is not — it has spawned and run successfully dozens of times today, with the gate's own
`"identity.spawn.blocked"` warning appearing **zero times** in a full day of `v0.55.18`
server logs, for any agent, not just this one.

**Confirmed, separately, on-disk consequence:** this agent's sessions have been writing to
the operator's global `~/.claude/projects/` home instead of the isolated per-agent
`CLAUDE_CONFIG_DIR` since ~2026-08-05 — exactly the outcome the gate exists to prevent.

## 2. Why this is higher priority than the original continuity complaint

If the gate genuinely isn't enforcing, credential isolation between agents — and between
an agent and the operator's own personal account — may not be reliably enforced right now
for any agent whose identity binding is empty/missing, not just this one. That's a
security/isolation property, not a UX nuisance. It also means any fix built directly on
top of findings from the two related docs below would be built on a foundation
(the gate) that isn't actually running — worth confirming that first.

## 3. Three related, previously-documented issues feeding into this

1. `docs/retro/retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md` — the
   original "auto-isolate" behavior for unbound agents was orphaned across five commits
   (2026-05-22 → 2026-07-14); today's gate is the eventual hardening response to it.
2. `docs/specs/REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md` — a prior
   instance of this same agent found that `db_agent_identity_links`/`db_accounts` resolve
   through a **per-channel-isolated** store by default on any non-`stable` channel, so a
   version/channel switch can silently empty an agent's binding row. Verdict at the time:
   "Not solved." Plausible explanation for why this agent's `identity_id` went blank
   sometime around 2026-08-05.
3. This investigation — traced the gate all the way through and found it structurally
   should fire but empirically never does, per live server logs.

## 4. Confirmed facts (DB state, queried directly against the live channel store)

```
db_agent_instances.identity_id = ''   (for this agent's running instance)
db_agent_identity_links: 0 rows total (in the live channel's own store)
db_agent_definitions.use_ambient_login = 0  (false — should make no difference; see §1)
```

## 5. What's needed to close this out (not done in this investigation — see retro §7)

Add a temporary `tracing::info!` at the top of `inject_identity_env_with_broker` logging
`block_id`, trigger one real spawn/respawn of an affected agent, and check whether it
appears in the log at all:

- **Doesn't appear at all** → the `agent_io.rs`/`input.rs` RPC handlers traced in this
  investigation are not the code path actually driving a live persistent controller's
  message delivery; the real dispatch path needs to be found from scratch.
- **Appears, returns `Ok`** → the bug is inside the function — it's reading different
  binding/provider data at runtime than the direct DB query in §4 saw.
- **Appears, correctly computes `Err`** → the bug is in how the caller handles that `Err`
  (swallowed somewhere before it blocks the spawn / surfaces in the pane).

This was deliberately not attempted in this investigation to avoid restarting a live,
in-use pane mid-conversation. Do this on a disposable/throwaway test agent first if
possible, to avoid disrupting a real agent's session.

## 6. Update — the gate function itself is proven correct; the bug is structural

Ran this module's own test suite (`cargo test -p agentmux-srv identity::resolver::inject`)
against the diagnostic build: **all 7 tests pass**, including
`inject_empty_identity_is_gated_same_as_blank` — a test that covers this agent's exact
production shape (`identity_id=""`, oauth-class provider, no binding, `use_ambient_login
=false`) and asserts the gate correctly returns `Err(SpawnGateError::MissingCredentials)`.

**This rules out a logic bug inside `inject_identity_env_with_broker`/`gate_oauth_failure`.**
Given this exact state, the function is proven — by its own test suite, run fresh against
the current code — to compute the correct "block" outcome. The discrepancy is therefore
structural: something prevents this correctly-computed `Err` from ever being reached or
ever taking effect on this agent's actual live respawn path in production, where the
identical state (confirmed via direct DB query, §4) does not block the spawn.

Also ruled out: a stale/duplicate DB copy from an older version. The channel this agent
actually runs under (`local-main-b28b7a-697d25a4`) only has a `versions/0.55.18/` folder —
no older `0.55.15`/`0.55.16` sibling for this specific channel to be silently reading from
instead (those version folders exist only under a *different* channel hash,
`local-main-b28b7a-01f827a1`, used by other agents — not this one).

**A cross-instance runtime test (spin up a fresh `task dev` instance, create a disposable
never-bound test agent, and drive it via the UI to check whether the diagnostic log lines
fire) was attempted and abandoned**: this agent's own `mcp__agentmux__UI*` tools are
confirmed scoped to "your own pane, cannot reach a different pane or agent's UI" — no
tool available in this session can drive a separate AgentMux window's UI. The dev instance
was built successfully (confirms the diagnostic-instrumented code compiles and the
`860fb0b6a` gate-hardening logic is present in a real `v0.55.18`-equivalent build) but was
torn down without a UI-driven test, to avoid burning further time on tooling that
structurally can't reach it from here.

Temporary diagnostic `tracing::info!` lines remain in
`agentmux-srv/src/identity/resolver/inject.rs` (marked `TEMP DIAGNOSTIC`, referencing this
doc) at: function entry, the no-instance-row early return, immediately before
`gate_oauth_failure` returns `Err`, and the final `Ok(())` return. These are cheap,
`info`-level, and safe to ship as-is if a restart happens before someone gets to remove
them — but should be removed once this is closed out.

**Remaining concrete next step, unchanged in substance from §5 but now more targeted:**
one real restart of an affected agent's pane (this one, or the disposable dev-instance
route if UI automation becomes available) with these diagnostic lines live, checking which
of the four DIAG lines actually appears in the log. Given the function is now proven
correct in isolation, the two live outcomes to distinguish are just:
- **No DIAG line at all appears** → `agent_io.rs`/`input.rs`'s `AgentSendCommand`/
  `AgentInputCommand` handlers are not the code path actually driving this agent's
  respawn (e.g., an internal `persistent.rs` retry — `retry_after_resume_failure` — may
  reuse a previously-resolved `env_vars`/config without re-invoking the gate at all,
  rather than re-entering the RPC handler). This would mean the gate only ever runs once,
  at a spawn's *original* triggering message — and if identity was still valid at that
  original moment (before whatever broke it later), every subsequent respawn of the
  *same* long-lived controller would keep reusing that stale-but-still-approved config
  forever, never re-checking.
- **The `Err`-about-to-return DIAG line appears, but the spawn proceeds anyway** → the
  `Err` is being swallowed or ignored somewhere between `inject_identity_env_async`'s
  return and `agent_io.rs`'s handling of it.

## 7. Non-goals / what this doc is not about

Not about Part B/C of `docs/specs/SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md`
(cross-instance session rehydration) — that gap is real but unrelated; this agent's
session file is in the *correct* isolated location and simply isn't being looked at
because the live process isn't using that `CLAUDE_CONFIG_DIR` at all.

## 8. Resolution (2026-08-21) — §5's recommended test was run; the gate works

§5/§6 asked for exactly one thing: a real restart with the `DIAG` lines live,
to see which branch actually executes. That test happened — not on this pane
(deliberately avoided disrupting a live conversation, as noted at the time),
but on a disposable `task dev` instance, opening an **existing** agent
("Lark") that was never touched by this investigation before.

**Result: the `DIAG` lines fired, and the gate returned `Ok` — correctly.**
Full log sequence:

```
DIAG: inject_identity_env_with_broker called for block 1f833386-...
WARN: instance 1f833386-... has empty/blank identity_id — falling through to
      the layer-3 gate instead of ambient creds. Legacy row or UI regression?
INFO: injected CLAUDE_CONFIG_DIR for oauth provider claude
      (identity=, account=a1990489-6de6-484a-9e20-83688c641524)
DIAG: inject_identity_env_with_broker returning Ok for block 1f833386-...
```

This is §5's second predicted outcome ("Appears, returns `Ok` → something
upstream is feeding it different (working) data than what I saw in the DB")
— confirmed, and the reason was found immediately: **`resolve_bindings_for_instance`
keys off `instance.definition_id`, not `instance.identity_id`** (this is
explicitly documented in that function's own doc comment, which this
investigation read earlier but didn't fully connect at the time). The "empty
identity_id" WARN is cosmetic — it does NOT mean "no binding." A direct query
of the correct store (`~/.agentmux/shared/identity-store.db` — **global**,
not the per-channel `objects.db` this investigation's §4 mistakenly queried)
confirms AgentY has a real, valid, `status: "valid"` binding:

```
db_agent_identity_links: (AgentY's definition_id, account a1990489-..., 'claude')
db_accounts: account a1990489-..., name "Claude (personal)", kind "oauth",
             secret_ref: {"backend":"oauth_config_dir","dir":"C:\Users\asafe\.claude"}
```

**So: the gate is not broken. §4's "0 rows" finding was a real fact about the
wrong database** (the per-channel `id_store`, which — per
`REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md`'s own
finding — legitimately does reset per channel) — **but the actual binding
lookup this gate performs reads the always-global `identity_store` instead**
(the PR #2632 fix), where the real binding has been sitting correctly the
whole time. Two different, both-real facts about two different databases got
conflated into one wrong headline conclusion. Worth remembering for next
time: confirm which store a piece of code *actually* reads before treating a
query against "a" store as decisive.

**What's actually still open, now correctly scoped:** the account itself —
"Claude (personal)" — has a `secret_ref` pointing at the operator's own real,
global `~/.claude`, not an isolated per-agent directory. That's real, and
it's why AgentY's (and Lark's) sessions land in the operator's personal
Claude Code history instead of an isolated one. Whether that's intentional
(the operator deliberately wanting these two agents to run on their own
personal login) or an unintentional/legacy binding is a product question,
not a bug — the credential-isolation gate enforced exactly what's configured.
Tracked as a follow-up issue rather than resolved here, since only the
operator can answer that.

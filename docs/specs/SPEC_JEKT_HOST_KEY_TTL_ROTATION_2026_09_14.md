# SPEC: 24h TTL / rotation for the host-tier jekt signing key

**Date:** 2026-09-14
**Status:** Implemented (this spec) — #3217
**Closes the gap tracked in:** `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md`
§non-goals ("Key rotation UX, revocation, or a management UI for per-agent
signing keys — needed eventually, sized separately once the core mechanism is
proven.") and `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` §5 ("Key
rotation/revocation UX (out of scope; host-tier HMAC keys don't have this
either yet — same gap, not new here.)").
**Consolidates:** this is the first spec to actually implement either of the
two "out of scope" notes above — it closes the host-tier half and explicitly
declines to close the LAN half yet (§4).

## 1. Problem

`db_agent_jekt_keys` (per-agent HMAC-SHA256 signing key, host-tier jekt
sender verification — `agentmux-srv/src/backend/storage/agent_jekt_keys.rs`)
and `db_agent_lan_keys` (per-agent Ed25519 keypair, LAN-tier —
`agent_lan_keys.rs`) are both minted once, on first spawn, and never rotate.
Verified directly against the code (not just the specs that flagged this as
a known gap):

- Neither table has an `expires_at` column (unlike several other tables in
  `migrations.rs`, e.g. `db_muxbus_pkce_state.expires_at`).
- `agent_jekt_key_ensure` / `agent_lan_key_ensure`: `if let Some(existing) =
  ...load(&key)? { return Ok(existing); }` — once a row exists, it is
  returned forever, unconditionally.
- The `migrations.rs` doc-comments above both `CREATE TABLE` statements say
  so explicitly: "minted on first use ... and never rotated automatically."
- Git history: both storage files have exactly one feature commit each
  (#2565, #2588) plus one unrelated lift-to-`agentmux-common` refactor
  (#3033) — no follow-up rotation work ever landed.

A key that never expires means a leaked `AGENTMUX_JEKT_KEY` (logged output,
a copied `.mcp.json`, a compromised host) grants indefinite forgery ability
for that agent's identity in every future `TRUST=host-verified` jekt, with
no time bound and no rotation path short of manually deleting the DB row.

## 2. Design: rotate lazily, at next mint, not by invalidating a live key

**TTL = 24h (`JEKT_KEY_TTL_SECS = 86_400`), checked in `agent_jekt_key_ensure`
only — `agent_jekt_key_load` (the verification-read path) is unchanged.**

Concretely: `agent_jekt_key_ensure(agent_id)`, which today is called from
exactly two `.mcp.json`-materializing call sites
(`agent_config.rs::inject_jekt_signing_keys_into_mcp_json`, shared by
`agent.open` and the Launch-button `WriteAgentConfig` path — see
`REPORT_JEKT_SIGNING_KEY_INJECTION_GAP_2026_08_16.md` for why those two call
sites must not drift apart) plus a test-only debug endpoint gated behind
`AGENTMUX_ENABLE_TEST_ENDPOINTS=1`:

- No row yet → mint and insert, unchanged from today.
- Row exists, `now - created_at < 24h` → return the existing key unchanged
  (today's behavior, for the common case).
- Row exists, `now - created_at >= 24h` → **rotate**: generate a fresh
  random key, `UPDATE` the row (guarded by `WHERE created_at = <the value
  just read>`, the same optimistic-concurrency shape as the existing
  `INSERT OR IGNORE` + re-read race guard, so two concurrent rotators for
  the same never-before-rotated agent_id agree on one winner instead of
  each minting a different key), and return the new key.

This only ever happens at the two spawn-time call sites — never on the
verification path (`agent_jekt_key_load`, read by `reactive.rs`'s sender
verification and `ui_handlers.rs`'s UI-automation auth). A currently-running
agent process keeps its own already-injected `AGENTMUX_JEKT_KEY` in its own
env for its entire lifetime and keeps signing successfully with it — nothing
server-side actively invalidates a key out from under a live process.
Rotation happens the next time that agent is *spawned* (normal Launch, or
respawn) after 24h has elapsed since its key was last minted/rotated.

### 2.1 Why lazy-at-next-spawn, not hard invalidation at T+24h

A hard-invalidate design (verification itself starts rejecting the key the
instant it turns 24h old, independent of whether the agent has respawned)
would force every long-running agent process into `TRUST=unverified` —
which CLAUDE.md's jekt rules treat as an **active red flag**, unconditionally
forced to `TIER=sensitive`/`ESCALATE=required` — the moment its own key
crosses 24h, even though nothing about that agent's identity actually
changed. There is no live key-refresh push to an already-running MCP server
process today (the key is only ever handed over via env injection at
spawn), so hard invalidation would have no companion mechanism to keep a
long session working and would turn "the key is old" into "every message
this agent sends now interrupts the human," which is a worse outcome than
the unbounded-lifetime status quo for any agent with a long session.

Lazy rotation still delivers the actual security property this spec exists
for — a leaked key's forgery window is bounded to "until this agent's next
normal respawn," not "forever" — without that regression. The trade-off
made explicit: an agent that is spawned once and left running for weeks
keeps using its original key for that entire session. This is a real
limitation, not an oversight; closing it requires a live in-process
key-refresh channel, which is a separate, larger piece of work (tracked as
a follow-up, not attempted here).

### 2.2 Why this needs no migration and no `expires_at` column

`created_at` already exists on both tables and is already always set to
`now_secs()` at mint time. TTL is computed as `now - created_at >= 86_400`
in Rust at read time — no schema change, no `OBJECT_SCHEMA_VERSION` bump.
This is also what makes the fix apply to **already-existing** rows for free:
every agent that was spawned before this shipped already has a `created_at`
timestamp from whenever its key was first minted, almost certainly more
than 24h in the past — so the very next time any of those agents is
spawned/relaunched, `agent_jekt_key_ensure` sees a stale row and rotates it
on that call, with no backfill step required.

## 3. What does NOT change

- `agent_jekt_key_load` and every one of its callers (`reactive.rs` sender
  verification, `ui_handlers.rs` UI-automation auth) — byte-for-byte
  unchanged. Verification still just reads whatever key is currently on
  file; it has no concept of "too old," only "matches" or "doesn't."
- `TRUST=host-verified` / `TRUST=unverified` / `TRUST=self-declared`
  semantics in `handler.rs` / `sanitize.rs` — unchanged. A successfully
  verified signature against a post-rotation key still renders
  `TRUST=host-verified`, exactly as it did against a pre-rotation key. This
  spec changes when a key is replaced, not what a valid signature proves.
- No CLAUDE.md jekt-rules changes needed: nothing in the `TIER=`/`TRUST=`/
  `ESCALATE=` decision tables changes behavior. (CLAUDE.md's own historical
  note about "host-tier HMAC keys don't have [rotation] either yet" is now
  stale for the host-tier half specifically; not rewriting that prose here
  since it lives in two other specs' "scope excluded" sections describing
  *those* specs' original scope at the time, not a live behavior table.)

## 4. Explicitly out of scope: LAN-tier (`db_agent_lan_keys`) rotation

The original ask was "24h TTL ... for all existing agents (across all our
local infra)," which could be read to include LAN-tier Ed25519 keys too.
**This spec deliberately does not rotate `db_agent_lan_keys`.** Reason,
verified against `lan_peer_pubkey_pins.rs`:

`db_lan_peer_pubkey_pins` is a trust-on-first-use pin: the first LAN public
key ever observed for a given `agent_id` is pinned **permanently** — no
expiry column, no re-pin RPC, and `lan_peer_pubkey_pin_get_or_set` explicitly
returns the *original* pinned key even when a caller supplies a different
one, precisely so a later, different key claiming the same agent_id is
treated as a mismatch (`lan_verified = Some(false)`) rather than silently
trusted. That mismatch path is, per CLAUDE.md's jekt rules, one of the three
cases "worse than unsigned" — unconditionally forced
`TIER=sensitive`/`ESCALATE=required`, no exception.

If LAN keys rotated on the same lazy 24h schedule as host-tier keys, every
LAN peer that had already pinned this agent's pre-rotation public key would,
after that agent's next respawn, start seeing every one of its LAN jekts as
an **active forgery attempt** — not a downgrade to unverified, a permanent
false-positive "someone is impersonating this agent" alarm, for every
peer that pinned before the rotation, with no existing mechanism to
re-pin and clear it. That is a strictly worse outcome than today's
never-rotates status quo, and shipping it silently as part of "add the TTL
that was already supposed to be there" would be exactly the kind of
unreviewed trust-layer change CLAUDE.md's jekt section warns against
trusting without independent confirmation.

Closing this properly needs a rotation-announcement / re-pin mechanism
first (e.g., a signed "my key is rotating, here is the new one, still
signed by the old key as proof of continuity" jekt, or an explicit
human-driven re-pin action) — sized separately, tracked as a follow-up, not
attempted in this change. `db_agent_lan_keys` rows keep their current
"minted once, never rotated" behavior; only `db_agent_jekt_keys`
(host-tier) gets the TTL in this spec.

## 5. Key files

- `agentmux-srv/src/backend/storage/agent_jekt_keys.rs` — `JEKT_KEY_TTL_SECS`
  constant, TTL check + rotation branch in `agent_jekt_key_ensure`, new
  tests.
- `agentmux-srv/src/backend/storage/migrations.rs` — doc-comment above
  `db_agent_jekt_keys` updated to describe the new rotation behavior (no
  SQL/schema change).

## 6. Testing

- `ensure_does_not_rotate_before_ttl_expiry` — mint, call `ensure` again
  immediately, same key returned (existing `ensure_is_idempotent...` test
  already covers the zero-age case; this is the same assertion made
  explicit against the TTL check specifically).
- `ensure_rotates_key_after_ttl_expiry` — mint, backdate `created_at` past
  `JEKT_KEY_TTL_SECS` via direct SQL (test-only), call `ensure` again,
  assert the returned key differs from the original and `created_at` was
  bumped to (approximately) now.
- `agent_jekt_key_load` after a rotation returns the NEW key, not the old
  one — proves the rotation is a real replacement, not an additional row.

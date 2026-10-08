# SPEC: each channel signs in to MuxBus on its own

**Date:** 2026-10-08
**Status:** implemented (#4477; repair and refresh leadership in #NNNN) — §6 lists what is not built.
**Author:** Camper, at the operator's direction
**Amends:** `SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md` (reverses it for MuxBus sign-in only),
restoring the per-channel half of `SPEC_MUXBUS_KEYCHAIN_PER_CHANNEL_2026_10_02.md` for every channel.
**Related:** `docs/retro/RETRO_MUXBUS_SIGN_OUT_SHARED_KEYCHAIN_RACE_2026_10_08.md` (why),
`SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md` (who receives a WAN jekt),
`SPEC_MUXBUS_CROSS_CHANNEL_DUPLICATE_DELIVERY_2026_07_04.md`,
`SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15.md`.

## 1. The decision

> Each channel on the same host needs to maintain isolation. A channel is the base entity, not the host.
> (operator, 2026-10-08)

The operator chose the narrow reading: **MuxBus sign-in is per channel**. Provider logins,
accounts, bundles, drones and Global Memory stay shared as `SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md`
made them.

## 2. Requirements

R1. **A channel is the unit of MuxBus sign-in.** Each channel except `stable` has its own saved sign-in,
its own tokens in the OS keychain, and its own cached per-agent cloud credentials.

R2. **Two channels on one host can be signed in at once**, to the same account or to different ones. One
channel signing in, refreshing or signing out never reads, writes or deletes another's.

R3. **`stable` keeps the host-wide sign-in** (`muxbus:global`, row `global`), so installed releases, which
all share the `stable` channel, stay signed in across upgrades.

R4. **Inside one channel, sign-in is safe across processes.** Two processes of one channel (two live
versions of `stable` during an upgrade, say) cannot interleave a save or tear a read.

R5. **A read that raced a write is not a sign-out.** A torn read is retried before it is believed.

R6. **Messages reach the right agent.** Whichever channels are signed in, a jekt addressed to an agent is
delivered to the one instance that runs that agent (§4).

## 3. Design

### 3.1 What a channel is

A channel is one build's own data folder, identified by `AGENTMUX_CHANNEL`: `stable` for released
builds, `local-<branch>-<hash>-<label>` for `task package`, `dev-<branch>-<clone>` for `task dev`.
Everything below keys on that value.

### 3.2 Where the sign-in lives

| Piece | `stable` (or no channel) | any other channel |
|---|---|---|
| Keychain tokens | `muxbus:global` | `muxbus:channel:<channel>` |
| Saved-login row, `db_muxbus_credentials.id` | `global` | `channel:<channel>` |
| Cached cloud credentials, `db_agent_credentials.agent_id` | `<agent>` | `<channel>/<agent>` |
| Cross-process lock file | `shared/muxbus-locks/muxbus_global.lock` | `shared/muxbus-locks/muxbus_channel_<channel>.lock` |

Where a channel's store is already its own file (`AGENTMUX_ISOLATED_AUTH=1` with an instance dir) the row
id and the credential keys stay as they were (`global`, unprefixed), so an isolated channel's existing
sign-in remains valid. Its keychain namespace is unchanged as well (it was already per channel).

The cached per-agent credentials belong to the account a channel is signed in as. They were keyed by
agent id alone in the shared store, so two channels signed in as different accounts overwrote and cleared
each other's. They are now behind the channel prefix; sign-out and account switch clear only the
signing channel's rows; deleting an agent removes its rows in every channel. Adoption
(`SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md` §5) no longer copies them from an earlier store: they are
provisioned again on first use.

A channel whose own tokens are missing still falls back once to the host-wide set, which a channel signed in
before tokens were scoped used, but only when that set's account (`sub`) is the one its saved row names.
Otherwise `stable`'s account could pair with this channel's row.

### 3.3 The cross-process lock

`muxbus_save_lock` serializes threads of one process. A save on Windows is about a dozen keychain writes,
and the keychain is host-wide. An OS advisory lock on a file beside the shared store (`flock` /
`LockFileEx`, the same primitive as `registry::LeaseStore`) now wraps `muxbus_save`, `muxbus_load` and
`muxbus_clear`, taken after the in-process lock. It waits up to five seconds; if it still can't take the
lock, the operation fails as a temporary error rather than running unlocked: a load is retried with
backoff, and a sign-in or sign-out reports an error to try again. The OS releases the lock when its
holder exits, so a crashed holder blocks nothing. The lock file is never deleted.

### 3.4 A torn read

The Windows read path retries a read that looks torn (mismatched generation stamps, some fields present and
others not, or a field caught mid-write with a chunk or its stamp missing) up to four times, 120 ms apart.
A keychain that cannot be read at all is still an error, not a tear. A build without the lock may be mid-save; the same read a moment
later is whole.

### 3.5 Repairing a tear that persists

A tear that persists is repaired without a sign-in when the refresh token's own field is whole (its chunks
and stamp complete). The load hands back that refresh token alone, so the next refresh asks Cognito for new
tokens with it and saves a whole set. Cognito doesn't rotate refresh tokens, so any save's token works.
Before saving, the refresh checks that the new tokens belong to the account the saved row names (`sub`); a
torn set that paired another account's refresh token is refused and needs a sign-in. A tear whose refresh
field is itself broken still means "sign in again".

### 3.6 One refresh at a time, and noticing a sign-in elsewhere

- **Refresh leadership.** A refresh takes a second cross-process lock, separate from the store's so reads
  aren't held up by the token request, and holds it across the request and the save (waiting up to 30 s).
  Under it, it loads the sign-in again; if another process has just refreshed it, it uses that and makes no
  request.
- **A parked channel looks again.** With nothing stored, or nothing usable, the subscriber used to wait for
  a sign-in in its own process. It now also re-reads the store every 60 s, so a sign-in by another process
  of the same channel (two live versions of `stable`, say) brings it back within a minute.

## 4. Who receives a message

Sign-in only decides whether a channel can talk to the cloud relay and as whom. It does not decide which
agent a message is for. That is decided per tier:

| Tier | How the target is chosen | Does per-channel sign-in matter? |
|---|---|---|
| Host (`DELIVERY=host`, and `channel` for another channel on the same host) | the host-wide live-agent registry under `shared/agents/reactive` | No: no cloud sign-in is involved |
| LAN | mDNS discovery and the peer's own endpoint, per instance | No |
| WAN | the relay delivers to the instance that holds the agent's lease; an instance is `host/channel` | **Yes**, see below |

On the WAN, a channel's sign-in is the account it speaks as. Cases:

1. **Different agents on different channels, same account.** Each channel pulls only for the agents it
   registered. Each receives its own. (Unchanged: before this change both channels were signed in as the
   same account.)
2. **The same agent on two channels, same account.** One instance holds the agent's lease
   (`SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md`); the other's pulls are fenced and it is shown as
   held elsewhere. The instance id is `host/channel`, so two channels on one host are two claimants.
   (Unchanged.)
3. **Different accounts on different channels.** This is new: before, all channels on a host spoke as one
   account. Different agent names separate cleanly. The same agent name on both is not decided by this
   spec and is not assumed to work.

A message sent by an agent on one channel to an agent on another channel of the same host does not go
through the relay at all (the host tier), so it is not affected by which account either channel signs in
as.

## 5. Consequences

- **Every channel but `stable` signs in to MuxBus once.** Each new `task package` build and each new
  `task dev` branch or clone is a new channel and asks for its own sign-in, the cost
  `SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md` removed. A channel that had been using the shared
  sign-in looks signed out after this change until it signs in; `stable` does not.
- The sign-in row and tokens of a retired channel stay behind in the shared store and the keychain.
  Channel pruning (`SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md`) is unbuilt and does not clean these.
- Older builds keep using the host-wide set and have no lock. While one runs beside a newer build of the
  same channel, a tear is possible; the retry (§3.4) covers the read side only.

## 6. Not built

- **Showing a torn or signed-out state to the user as such.** Today it reads as "not connected"; #4489
  adds a delivery state for it.
- **Pruning** a retired channel's sign-in.
- **One agent name under two accounts** (§4 case 3): untested, tracked outside this repo.

## 7. Tests

- Namespace, row id and credential prefix per channel and for `stable`, including an isolated store
  keeping its existing keys; two channels never share a keychain entry.
- The lock: a second holder waits, then fails the operation if the lock is still held; it is free once
  released; locks are per namespace.
- A torn set keeps a whole refresh token and drops a broken one (live, real keychain); a refresh-only
  sign-in counts as refreshable; a refresh for another account is refused.
- A torn read is retried until it settles, gives up after the allowed tries, and an absent sign-in is not
  retried.
- The host-wide fallback adopts only the same account's tokens.
- Adoption copies accounts but no cached credentials.
- Cached credentials: the host-wide clear removes only unprefixed rows; a channel clears only its own;
  deleting an agent removes its rows in every channel.
- Live, against the real Windows keychain (ignored in CI,
  `concurrent_processes_never_tear_a_live_sign_in`): four processes each save and read one namespace 40
  times. Without the lock 35 to 59 of the 160 reads were torn, mixed or failed across three runs; with
  it, none. A save took up to about 0.7 s under that contention.
- Manual, with two `task dev` clones: sign both in, refresh both at once, sign one out; the other stays
  signed in, and a jekt to an agent on each arrives in its own pane.

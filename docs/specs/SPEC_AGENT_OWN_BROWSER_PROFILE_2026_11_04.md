# SPEC: every agent browses with its own named, persistent profile (2026-11-04)

**Status:** active — approved by the operator (2026-11-04, via AgentY); built in #17136
**Author:** Camper-2@narko, 2026-11-04, at the operator's direction

Task #17135; the build is #17136. Author: Camper-2. Browser lane: AgentX is told.
The operator's words (2026-11-01, relayed by AgentY): "lets add guidence that agents should always open their browser with a profile sharing their own name, and they would always use that. that way logins are always present."
The operator green-lit this spec's recommendations (2026-11-04, relayed by AgentY), so each decision below is made, not asked. Nothing here costs money or needs a new external account.

## Recommendation

An agent's `Browser open` with no `profile` browses as a profile of its own, created the first time it is needed, named after the agent and keyed by its agent id. Its cookies and sign-ins stay on disk across panes, restarts and upgrades, so the next pane is already signed in. This holds for native and container agents alike. Everything else about `profile` stays: `"incognito"` still gives a fresh jar, and a user profile still needs the user's "Agents may use it". A native agent can still ask for `"personal"` (the user's own sign-ins) by name, as today.

Guidance alone cannot do this (the task's reasons): the default is Personal for a native agent and Incognito for a container agent, and a profile can be created only by the user. Both change here.

## How it works today (main, 2026-11-04)

- `browser-profiles.json` in the shared data folder holds `{id, name, color, created_at, agents_allowed}`; each profile's cookies live in the CEF cache at `profile-<id>`. A pane's profile is its block meta `browser:identity`: absent = Personal, `incognito:<uuid>`, or `profile:<id>`.
- `browser_identity.rs`: `identity_for_agent` resolves the `profile` an agent asked for; `for_agent_open` applies a container agent's Incognito default (`requested_by_container`) and refuses "personal" for it; `for_agent_pane` sets the owner and the identity.
- `app_api/browser_profiles.rs` refuses create, update and delete from anything but the user's window (`not_for_agents`, a test asserts it).
- Settings → Browser (`browser-section.tsx`) lists the profiles, each with name, colour, "Agents may use it" and Delete.

## Decisions

1. **One profile per agent, found by agent id.** A new field `owner_agent: Option<String>` on `BrowserProfile` (absent for the user's own, so old files read unchanged). The profile id is `agent-<agent id>`, so two concurrent first opens cannot make two. Its `name` is the agent's id (its route name, e.g. `camper-2`) as of each open, so a rename shows without moving the data; showing the display name instead needs an agent lookup in this path and is left for later. The row is created inside srv, under the existing `WRITING` lock, by `for_agent_open` when it is first needed: no create call is exposed to agents and no operator check is relaxed for the user's own profiles. The "agents can't create profiles" rule is lifted only in this one internal path, for the calling agent's own id.
2. **Default.** With no `profile` (or `profile: "own"`, accepted as an alias), every agent gets its own profile. `incognito` and `personal` are unchanged (container agents are still refused `personal`). A name that matches the agent's own profile resolves to it before any user profile of the same name.
3. **Access.** An agent may use a profile where `owner_agent` is its own id, or any profile with `agents_allowed`. Another agent's profile is not offered and not resolvable by that agent, unless the user switches "Agents may use it" on for it. The answer to `open` already says whom the pane browses as (`browsing_as`); for the agent's own profile it says "your own profile (<name>)" and, when the jar is new, "no sign-ins yet: sign in once (hand off for a code, or `signin` with a Vault entry) and it stays".
4. **Agents that relied on Personal.** They move to their own profile on their next `open`; panes already open keep their identity. The cost is one sign-in per site per agent, once. We recommend this: Personal as a silent default means any agent can act as the user on every site they are signed in to, and it does not scale to many agents. An agent that truly needs the user's sign-in asks for `profile: "personal"`, and the pane's header already shows whose it is. The Browser tool's description says so.
5. **Settings → Browser.** A second list, "Agents' profiles", under the user's: the agent's name, "Other agents may use it" (the existing flag) and Delete. Delete is the sign-out: it closes the panes using the profile, drops the jar, and the agent gets a fresh profile the next time it opens a browser. A separate "sign out, keep the row" is not built (it would need a new host call for no gain over Delete). A user-created profile that happens to share an agent's name is told apart by an "agent" badge.
6. **Edge cases.**
   - *Two panes of one agent at once:* they share the one jar, as two tabs of any profile do.
   - *A forked agent:* the fork has a new agent id, so it starts with an empty profile. Copying a profile folder that CEF may have open is unsafe, so it is not offered; the user signs in again, or lets the fork use the original's profile with "Let other agents use it".
   - *A deleted agent:* its profile stays, labelled "agent deleted", until the user deletes it in Settings (no silent purge of sign-ins, no silent keep forever: the Settings row shows its size).
   - *A renamed agent:* the data stays; the row's name follows.
   - *A container agent:* its profile is stored on the host with the browser, under its agent id; it is separate from Personal and from other agents', which is the isolation containers were given Incognito for. A container agent still never gets `personal`.
   - *Another machine:* profiles are per machine, as the cookies are; the same agent on a second machine starts empty there.
   - *The cap:* Incognito's cap of 8 jars does not apply; one named profile per agent is bounded by the number of agents.
7. **When it ships.** Update the Browser tool's `profile` text in `tool_schemas.rs`, and add the short Global Memory entry the task asks for ("open your browser with your own profile; it's the default"; the build's PR text gives it to the steward on each host, as Global Memory isn't synced).

## What it touches

`crates/srv/src/backend/browser_profiles_store.rs` (the field, `ensure_agent_profile`), `crates/srv/src/server/browser_identity.rs` (`for_agent_open`, `identity_for_agent`, `requested_by_container`, `check_agent_may_use`), `app_api/browser_profiles.rs` (delete stays user-only; add "sign out" and the agents' list to the result), the RPC types and their generated stubs, `frontend/app/view/settings/sections/browser-section.tsx`, `crates/mcp/src/tool_schemas.rs`. No CEF-host change if "sign out" reuses the delete path.

## Tests

- Rust unit tests (CI): the first open creates `agent-<id>` once, a second open (and a concurrent one) reuses it; a rename changes the name and not the id; agent A cannot resolve agent B's profile, can after `agents_allowed`; a container agent with no `profile` gets its own profile and still cannot ask for `personal`; an old `browser-profiles.json` without `owner_agent` loads; the create/delete RPC still refuses an agent.
- One live check on a dev build (not on narko, per the build ban): sign in on a local test site in an agent's pane, close the pane, `open` again, and the site is still signed in; "Sign out everywhere" then shows it signed out.
- Frontend: a vitest for the agents' list rows (badge, "agent deleted", Sign out).

## Risks

- Existing agents lose the user's sign-ins by default. That is the intent; the cost is stated in decision 4, and the explicit `personal` stays.
- Sign-out while a pane is open drops the pane: the confirmation names how many tabs close.
- #18111 (retake and commandeer) interacts: a commandeered pane keeps the profile it has; the commandeering agent does not inherit access to it beyond that pane. See that spec.

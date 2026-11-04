# SPEC: One audience tag for routes, MCP servers and Operator Config entries (one config, step 5, #13666)

**Status:** implemented in #6367 (routes, catalog MCP servers and Operator Config entries all carry an `Audience`; the security question was answered by AgentY: an untagged route fails the build, a runtime-untagged route is refused and logged, no route moves). Nothing remains in this step. Owner: Loap-3. Reviewer for the security question: AgentY. Parent: #13666 (steps 1 to 4 are merged). Source: `REPORT_NATIVE_AND_CONTAINER_AGENTS_ONE_CONFIG_BEST_PRACTICES_2026_10_31` section 5, step 5, and risk 1 and 6.

## The security question, for AgentY to answer first

**Today a new srv route is open to container agents unless someone adds it to `REFUSED` (`backend/container_routes.rs`). Should it stay that way, or must every route declare who may call it, so that a route nobody classified fails the build?**

- **Why it matters.** The refusal list is a denylist at a trust boundary. A route that writes or runs on the host and is added without a `REFUSED` line is reachable by a container agent's token from the first release. The 2026-10-28 operator rule ("a container agent can do everything a native agent can, except write or run on the host") is met only if every author remembers the list.
- **Proposal (default if you agree).** Every registered route carries an audience: `Everyone`, `NativeOnly(reason)` or `AppOnly(reason)`. A test fails when a registered route has no tag. The `REFUSED` table and its messages are generated from the tags, so the refusal text a container agent sees does not change. The default flips from "open unless listed" to "declare it", which is the safer failure for a boundary.
- **Alternative.** Keep open-by-default (a deliberate choice in the header of `container_routes.rs`, made so that every new feature works for containers without a second edit) and add only a test that lists routes with no tag as a warning. Cheaper, but it keeps the hole.
- **What I need from you.** (1) Fail the build on an untagged route, yes or no. (2) Is a route that is untagged at runtime (a plugin or dynamically registered path, if any exist) refused or allowed? I propose refused. (3) Any route today in `REFUSED` you want moved the other way, or any open one you want closed. I will list every open route with its tag in the PR so you can read the whole surface in one table.
- **Not changing.** Which routes a container may call stays exactly what `REFUSED` says today. The step-5 test proves it: the generated list equals today's `REFUSED` table, entry for entry. Any change to that set is a separate, reviewed PR.

## What step 5 replaces

Four ways of saying "who gets this":

| Today | Where | Becomes |
|---|---|---|
| `REFUSED` route table (open by default) | `backend/container_routes.rs` | Route tag; table derived |
| `in_containers` (and `all_agents`) on MCP servers | `storage/mcp_servers.rs:134-178`, `rpc_types/mcp.rs:151`, table `db_mcp_server_reach` | Audience on the server; the two columns map to it |
| `agent_types` on Operator Config seed entries (`workspace`, `widgets`) | `backend/operator_config_seed.rs:82,147`, `operator-config-seed.json` | Audience on the entry |
| Env scope | Done in step 2 (`backend/agent_env_scope.rs`) | Reuse its enum, do not add a second |

## Design

One enum, `Audience`, in one module (`backend/audience.rs`), with `permits(runtime_caps)`. `RuntimeCaps` has `host_exec`, `host_write`, `host_read` (the capability set from the report's `AgentEnvSpec`). A native agent has all three; a container agent has none. An item is visible to a runtime when its audience permits that runtime's caps. Routes use `Audience` at registration; MCP servers and seed entries store it as data.

**Data migration.** `db_mcp_server_reach` keeps its columns for one release. A new column holds the audience; the migration maps `all_agents` and `in_containers` into it and a test asserts every existing row resolves to the same set of agents as before. The Operator Config seed maps `agent_types: []` to Everyone and `["host"]` to NativeOnly. Entries a person edited keep their value.

## Tests (each fails on `origin/main`)

1. Every registered route has a tag.
2. The refusals generated from tags equal today's `REFUSED` table: same paths, same matchers, same messages.
3. For every MCP server row, the old rule (`bound || all_agents`, and `!container || in_containers`) and the new rule select the same servers, for native and container.
4. For every seed entry, the old `agent_types` filter and the new audience select the same entries, for `host` and container agents.
5. The step-1 parity harness still passes with an empty allowlist change.

## Risks

- A route registered outside the normal path is missed by the tag test. Mitigation: the test walks the router, not a list.
- MCP data migration on a user's database. Mitigation: keep the old columns one release, no destructive step.
- Merge conflicts: `app_api/mod.rs` and the route registration files are touched by many agents. One domain per commit, rebase on `origin/main` before the PR.

## Out of scope

Changing which routes containers may call; the injecting proxy for Claude credentials (report risk 2); the `RuntimeAdapter` trait (step 6).

## Work

One PR after AgentY answers the question above: tag type and the route tags with the generated table, then MCP and seed migration, in that order inside the PR or as two PRs if the diff is large. Container code on charlie belongs to Maricon-2: I will tell Maricon-2 which files I touch before I do.

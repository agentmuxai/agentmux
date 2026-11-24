# Spec: bring your own harness: a custom ACP harness registered by a manifest

**Status:** active: approved by the operator on 2026-11-20 with every recommendation in section 10. Steps A and B (section 9) are built in the pull request that adds this file (build #29415); C to F remain.

Task #29414 (spec; the build follows it, #29415). Agent5, 2026-11-12, **revised 2026-11-20** against `origin/main` at v0.60.36 (the registry now lives in `crates/srv-core/src/backend/`; the changes are marked **[REV]**). Related: #29362 (harness curation, its `tier` field), #27459 (any model on any harness).

The operator's ask (relayed 2026-11-08): users can bring their own harness, not only their own keys, logins and models.

## 1. The ask, and the answer in one page

Today a harness is a Rust constant compiled into the release, so adding one is a code change and a release. Nine of the nineteen already speak ACP over stdio through one generic controller, so a harness that speaks ACP can be described by data.

**Proposal:**

- A **manifest** (`harness.json`, shaped like a widget's `widget.json`) in a folder under the machine's shared data, one per custom harness. srv reads them and adds them to a second, runtime layer of the provider registry. No release.
- **ACP first.** The controller that runs the nine existing ACP harnesses runs a custom one unchanged. A terminal-only harness (like aider and crush today) is a small second step (decision 3).
- **A custom harness is arbitrary code run unattended with an agent's authority.** So registering one shows the person what will run (the resolved binary, its hash, its arguments, what it will be given) and records their approval against that hash. A changed binary asks again. Section 5 is the full model.
- **Container first where it can be.** A manifest whose binary comes from an install source (a pinned npm package, or a download with a SHA-256) can run in a container agent, which is the sandbox. A manifest that points at a binary already on the host runs only as a native agent.
- It shows in the picker as a **Custom** tier beside the #29362 tiers, and is never read from a repository or workspace folder.

## 2. What exists today (verified)

- **The registry is compiled in.** `static REGISTRY: LazyLock<HashMap<&'static str, &'static ProviderConfig>>` (`backend/providers.rs:335`); `get_provider(id) -> Option<&'static ProviderConfig>` (`:535`) and `all_providers()` (`:551`). **[REV]** `get_provider` looks the id up directly and then through an `ALIASES` map, so a custom layer asked after the built-ins also has to answer for ids the aliases resolve (a custom id is never an alias target). Every `ProviderConfig` field is `&'static`: the id, the command, the launch arguments (`launch_args`, `persistent_launch_args`), the resume flag, the session-id field, the styled-output format, the optional `app_server` block, the auth config-dir variable and folder name, the npm package and pinned version, the supported vendors, the startup-instructions filename and the list of native instruction sources it can read. There are 19 providers. **[REV]** The manifest in section 3 covers a subset of those fields on purpose; the rest take the generic ACP defaults.
- **Nine use ACP** (`controller_type: ControllerType::Acp`): cline, copilot, dsh, goose, grok, kilo, openclaw, opencode, pi. The controller is `backend/blockcontroller/acp.rs`, `acp_spawn.rs`, `acp_mcp.rs`: it starts the process, sends `initialize`, creates or restores a session, sends each turn as `session/prompt`, reads `session/update`, and auto-answers the harness's permission requests (`allow_always`, else `allow_once`, else cancel). It does not offer the harness a file or terminal capability, so the harness uses its own tools.
- **The ACP controller is generic** except one hook: `acp_prelaunch.rs` matches the provider id for `grok` (trusting the folder) and `dsh` (a runtime check and a patch). A provider with no match gets nothing special.
- **MCP servers reach an ACP harness through `session/new`**, from the `.mcp.json` written at launch, with the `agentmux-mcp` entry (the agent's MuxAPI) added. A harness that speaks ACP gets the whole MuxAPI that way.
- **Installs:** an npm package at a pinned version into `shared/cli/<provider>/<version>/`, a download with a SHA-256 per platform (`backend/cli_download.rs`, **[REV]** today goose, grok, muse and antigravity; its asset table is compiled in as `&'static` data, so a manifest's `download` source needs the downloader taught to take an owned asset, which is step E's work), a Python venv, or a binary already on the PATH (`backend/cli_install.rs`).
- **Startup instructions:** each provider names the one file AgentMux writes (`CLAUDE.md`, `AGENTS.md` or `GEMINI.md`; **[REV]** pi writes `AGENTS.md` and only reads `.pi/APPEND_SYSTEM.md` and `.pi/SYSTEM.md` as native sources) in the agent's working folder, or none.
- **Auth:** a provider names a config-dir environment variable (`auth_config_dir_env_var`) and a folder name under `shared/providers/`. Accounts link by provider id.
- **The frontend keeps its own copy of the provider list** (`frontend/app/view/agent/providers/catalog.ts`, with `provider-access.ts` for the Free and Local tags), mirrored from the Rust registry. The picker reads that, not srv.
- **The route catalog is compiled in too** (`backend/model_gateway/catalog.rs`: `Model` and `Route` with `&'static` ids, prices read on a stated date). It is the same root gap: a new route model needs a release.
- **No extension point exists** for a harness. The nearest patterns are widgets (a folder with a manifest; **[REV]** an install is now the sensitive operation `widget_install` in `backend/sensitive_ops.rs`: it names fixed facts (the widget, its publisher, its permissions), a super agent reviews it where there is one, the person sees a 30 s window with a veto, and the window doubles when no super agent is there) and MCP servers (a user-written entry in `.mcp.json`).

## 3. The manifest

`harness.json`, `manifestVersion: 1`. One folder per harness: `<machine shared data>/harnesses/<id>/harness.json`, with an optional icon file beside it.

| Field | Meaning | Rule |
|---|---|---|
| `id` | `custom-<name>`: lowercase letters, digits, `-` | The `custom-` prefix keeps it from ever shadowing a built-in id. |
| `name`, `icon` | Display name; an image in the folder (PNG, SVG, 64 KB) or a built-in glyph name | |
| `protocol` | `"acp"` | The only value in v1. |
| `command`, `args` | The program and its argument list | No shell: an argument array, never a command string. `command` is an absolute path, or a bare name resolved once at registration and stored as the absolute path. |
| `env` | Fixed environment variables | Not secrets. No `AGENTMUX_*` and no `PATH`. A secret comes from `auth`. |
| `auth` | `{kind: "none"}`, `{kind: "key", envVar}` or `{kind: "config-dir", envVar}` | A key is a Connectors account whose secret is brokered into the variable per use; a config dir is the per-identity folder, as `auth_config_dir_env_var` does today. A secret is never in the manifest. |
| `startupInstructions` | A file name, or `null` | A plain file name: no separator and no `..`. AgentMux writes the agent's Global Memory into it, as for the built-ins. |
| `vendors` | Model vendors it uses | Descriptive, as `supported_vendors`. |
| `baseUrlEnvVar` | The variable that redirects its model endpoint, if it has one | Optional. Present: a route can serve it (#27459). Absent: it uses its own login only. |
| `install` | `{kind: "path"}`, `{kind: "npm", package, version}` or `{kind: "download", urls: {<os-arch>: {url, sha256}}}` | `npm` needs an exact version. `download` needs a SHA-256 per platform, keyed `<os>-<arch>` (`windows-x86_64`, `macos-aarch64`, `linux-x86_64` and the like) and an https address. With `npm` or `download`, `command` is a bare name: AgentMux knows where it installed it. `path` is native only. |

Validation runs in srv on load and on every change, and refuses the manifest (naming the field) on any broken rule. A manifest folder larger than 1 MB is refused. The harness is identified for the person by name, resolved binary path and SHA-256.

## 4. How it joins the registry at runtime

- `providers.rs` gets a second layer: a map of custom definitions behind a lock, filled from the harness folders at srv start and on a `Harness` RPC call (register, remove, reload). `get_provider` asks the built-ins first, then the custom layer. A custom id never wins over a built-in (the prefix guarantees it).
- **The `&'static` problem.** Every caller takes `&'static ProviderConfig`. Two ways: leak one `ProviderConfig` per distinct manifest (a few hundred bytes; keyed by the manifest's content hash, so a re-read of the same file leaks nothing, and a cap of 32 custom harnesses bounds it), or change the registry to hand out `Arc<ProviderConfig>` and touch every caller. I recommend the leak with the hash key and the cap: it changes no caller (decision 4).
- **What a custom harness gets by default:** the ACP controller, no prelaunch hook, no native-history link (the identity layer's `provider_class` has no arm for it, as for qwen and kimi today). So continuity is ACP's own session restore; the transcript and history are AgentMux's, recorded from the stream, so History and the Audit view work as for any ACP harness. Native-history import and Take over from native files do not.
- **The frontend** keeps its compiled list for the built-ins and gets the custom ones from a new srv call (`harness:list`, the same shape as a catalog entry), merged after them. The parity test between the two compiled lists does not cover the custom layer.
- Manifests are read only from the machine's shared harness folder. **Never from a repository or an agent's working folder**: a manifest that arrives with a clone would be a supply-chain hole (clone, open the folder, run its binary).

## 5. Trust and sandboxing

A custom harness is code the person chose, but it runs without them watching, with an agent's authority, and others can pick it later. The model:

1. **What it can reach.** The same as any agent: the `agentmux-mcp` entry in `session/new` gives it the agent's MuxAPI under the agent's own key (Tasks, Message, Presence, Memory, Browser, Vault requests with their usual approvals). Registering a harness grants nothing beyond what an agent of that kind has, and the consent text says so. It also reads the agent's prompts and its Global Memory (written to its startup file), and the manifest's `env`. It can make its own network calls; AgentMux cannot see them.
2. **Consent on registration.** The person is shown: the harness's name; the resolved absolute binary path and its SHA-256; the arguments; the environment names (not values); the auth kind; "runs as you on this machine" for a native agent, or "runs inside the agent's container" for a container one; and the list in item 1. Approval is recorded against the manifest hash and the binary hash. **[REV]** This is built on the widget install's route, not beside it: a new sensitive operation, `harness_register`, in `backend/sensitive_ops.rs`, with a fixed set of facts (the harness id, the binary's SHA-256, the auth kind, whether it runs native or in a container), a super agent's review where there is one, a 30 s window with the person's veto, and the doubled window when none is there. The sensitive-operation banner carries only those named facts, so the **full consent text above is a panel** inside the pane or the Toolchain pane that asked (never a window modal), and the approval is the operation's, recorded against the two hashes. A person who registers a harness from the Toolchain pane themselves approves with their own click in that panel. It is not a rule that "only the person may": a native agent can already run any command through its shell, and an agent may request the registration; the named approver decides (decision 1 asks whether the prompt applies to a native agent's own registration at all).
3. **A changed binary asks again.** The hash is checked at every start. A change (an update, a swap) refuses the start and shows the same prompt.
4. **Container first.** A container agent runs the binary inside its own container and volume, which is the real sandbox: the harness reaches the host only through the container's mounts and the MuxAPI broker. That needs a binary the container can get: `npm` or `download` with the pin. A `path` harness is a host binary, so a **container agent may not register one**; the refusal says why (a host-exec reason) and what to do instead (use an `npm` or `download` source, or ask the person to register it).
5. **A harness that does not answer.** `Harness validate` starts the binary (after consent), sends `initialize`, reports the protocol version and whether it can load or resume a session, and stops it. No prompt is sent. The result is shown in the consent panel, so a program that is not an ACP harness is caught before any agent uses it.
6. **Removal** deletes the manifest and stops its running agents (with the usual stop window).
7. **Auth.** A key or config dir is the person's Connectors account, brokered per use as for the built-ins; the manifest never holds a secret.
8. **Audit.** Registration, approval, a changed-hash refusal and removal go to the Audit view under the agent or person that did them.

## 6. The picker and the tiers

- A fourth tier beside #29362's `first_class` and `preview`: **`custom`**, shown with a Custom tag, only on a machine that registered it, with Edit and Remove in the Toolchain pane. Its install state is the same probe as the built-ins.
- Custom harnesses never appear in a container image's preinstalled list; they install at agent start from their pin.
- The picker's Free and Local tags do not apply (nothing is known about a custom harness's billing).

## 7. Relation to #29362 and #27459

- **#29362 (curation):** the demote list (aider, cline, crush, opencode, kilo, openclaw, dsh, muxcode) can stay compiled as preview for now. Five of the eight (cline, opencode, kilo, openclaw, dsh) are ACP, so each could later be a **shipped sample manifest** in a catalog repo, as the widgets catalog is, instead of Rust constants. That would let the long tail be demoted and kept without a release and without stranding a user: anyone can register the same manifest themselves. It is a later step, not v1 (decision 6).
- **#27459 (any model on any harness):** a custom harness with `baseUrlEnvVar` can be served by a route like a built-in. The route catalog is compiled in too; a runtime catalog layer for routes is the same kind of fix as section 4 and could share the loader. Out of this spec.

## 8. MuxAPI

A `Harness` tool with actions `list`, `validate`, `register`, `remove` (and `reload`). Available to container agents by default, with the one refusal in section 5 item 4. `register` from an agent raises the `harness_register` sensitive operation (section 5 item 2) and shows the consent panel; it does not itself approve.

## 9. Build plan (the build task, after approval)

| Step | What | Size |
|---|---|---|
| A | Manifest type, validation and its tests (every rule in section 3, a refusal names the field) | S |
| B | Custom layer in `providers.rs` (hash-keyed leak, cap, built-ins first), loaded at start; the ACP controller runs it unchanged; tests that a custom id never shadows | M |
| C | `harness:list` for the frontend, the Custom tier in the picker and Toolchain pane, Remove | M |
| D | The `harness_register` sensitive operation and its consent panel, binary hash pin and re-check at start, the `validate` probe, Audit entries | M |
| E | `Harness` MuxAPI tool, container sources (`npm` and `download` installed inside the container), the host-exec refusal | M |
| F | Terminal protocol (decision 3) and shipped sample manifests (decision 6) | S, later |

## 10. Decisions for the operator

1. **Does registering need a person's approval for a native agent?** A native agent can already run any command, so a prompt does not stop it. I recommend yes anyway, once per (manifest, binary hash): it matches the widget install (`widget_install`, a named approver and a veto window), it catches a swapped binary, and the prompt is what tells the person the harness gets an agent's MuxAPI. The alternative is no prompt for a native agent's own registration and a prompt only for a harness another agent or person will run.
2. **Container first, or host path first?** I recommend container first (npm or download with a pin), with `path` native only, as in section 5.
3. **Terminal protocol in v1?** A terminal harness (aider, crush) needs no protocol, only a command. I recommend after v1, since ACP covers the ones that matter and a terminal harness has no MuxAPI, no history and no startup injection worth a manifest.
4. **Leak or `Arc`?** I recommend the hash-keyed leak with a cap of 32.
5. **The `custom-` id prefix.** I recommend it: it guarantees no shadowing and makes a custom agent recognisable in the Swarm and Audit.
6. **Move the ACP preview harnesses to shipped manifests later?** I recommend yes, as its own spec once this is built.
7. **Manifests from a repository.** I recommend never, as in section 4.

## 11. Not covered

- A harness that speaks neither ACP nor a terminal (MSP, an app-server protocol): the built-in controllers stay compiled.
- Signing a manifest with a publisher key (the widget SDK has it): worth adding if manifests are shared.
- A catalog or marketplace of manifests.
- Billing or quota for a custom harness (see the subscription report for #29360).
- Whether any named harness is ACP-compatible beyond the nine in section 2.

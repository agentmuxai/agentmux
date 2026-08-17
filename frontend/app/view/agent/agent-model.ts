// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, createEffect, onCleanup } from "solid-js";
import { BlockNodeModel } from "@/app/block/blocktypes";
import type { PaneVoiceHandle } from "@/app/hook/useVoiceInput";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { atoms, getApi, WOS } from "@/app/store/global";
import { SignalAtom } from "@/util/util";
import { AgentViewWrapper } from "./agent-view";
import { buildAgentPaneIcon } from "./components/AgentPaneIcon";
import { PROVIDERS, resolveProviderAlias } from "./providers";
import { resolveVendorEnvOverride } from "./providers/vendor-env";
import { Logger } from "@/util/logger";
import { buildInstanceSlug } from "./defaults/instance-slug";
import type { LaunchOverrides } from "./components/AgentLaunchModal";
import { readActivitySummary } from "@/app/store/activitySummary";
import { buildConfigFiles } from "./agent-config-builder";
import { checkNodejsForProvider, agentmuxHome, resolveCliDir, resolveEffectiveLaunchProvider, resolveInitialRuntimeConfig } from "./agent-launch-env";
import { realAccountIdOrEmpty } from "./identity-carry-over";
import { refreshAccountCache } from "@/app/view/identity/identity-model";
import { dimAgentColor, isValidAgentColor, pickAgentColor } from "./agent-color";
import { parseSeedZoom } from "./agent-zoom-seed";
import { HISTORY_TAB_FOR_META_KEY, openOrFocusHistoryTab } from "./open-history-tab";

export class AgentViewModel implements ViewModel {
    viewType = "agent";
    blockId: string;
    nodeModel: BlockNodeModel;
    blockAtom: SignalAtom<Block>;

    viewIcon: () => string | IconButtonDecl;
    viewName: () => string;
    setViewName: (name: string) => Promise<void>;
    viewText: () => string | HeaderElem[];
    viewComponent: ViewComponent;
    noPadding: () => boolean;
    endIconButtons: () => IconButtonDecl[];
    nodejsError: string | null = null;

    // Callback wired by AgentPresentationView on mount so the title-bar
    // button can open the pane-scoped Stash modal without holding a
    // SolidJS context in the model. Replaced the former separate
    // _openIdentityModal / _openMemoryModal pair (Phase 3 slice 1 — one
    // "Stash" icon opens a unified tabbed modal). Named "Stash" (not
    // "Armory") to distinguish it from the global Armory pane — see
    // docs/reports/REPORT_ARMORY_STASH_NAMING_2026_07_27.md.
    _openAgentStashModal: (() => void) | null = null;

    // Voice-input target ref. AgentFooter populates this on mount with a
    // textarea-backed handle (and clears it on unmount). The exposed
    // `voiceHandle` accessor below delegates to whatever's current —
    // before AgentFooter mounts (or after it unmounts) it's a no-op.
    voiceTargetRef: { current: PaneVoiceHandle | null } = { current: null };

    _activityFlash: () => boolean = () => false;

    voiceHandle = (): PaneVoiceHandle => ({
        appendFinal: (text: string) => this.voiceTargetRef.current?.appendFinal(text),
        setInterim: (text: string) => this.voiceTargetRef.current?.setInterim(text),
    });

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);
        this.viewComponent = AgentViewWrapper as any;

        // Flash signal: set true briefly when the activity summary changes to a
        // new non-empty value. Compare to previous so unrelated meta writes
        // (status, agentName, etc.) during an active turn don't trigger
        // spurious flashes.
        const [activityFlash, setActivityFlash] = createSignal(false);
        let flashTimer: ReturnType<typeof setTimeout> | undefined;
        let prevActivity: string | undefined;
        createEffect(() => {
            const activity = readActivitySummary(this.blockAtom()?.meta);
            if (activity && activity !== prevActivity) {
                clearTimeout(flashTimer);
                setActivityFlash(true);
                flashTimer = setTimeout(() => setActivityFlash(false), 600);
            }
            prevActivity = activity;
        });
        onCleanup(() => clearTimeout(flashTimer));
        this._activityFlash = activityFlash;

        // Drive the pane's title from block meta — launching an agent sets
        // `agentName` / `agentIcon` and the frame title automatically picks
        // them up via the blockAtom subscription. Before this, the title
        // was the literal string "Agent" regardless of which agent ran.
        // See SPEC_AGENT_PANE_FOLLOWUPS item #8.
        this.viewIcon = (): string | IconButtonDecl => {
            const meta = this.blockAtom()?.meta;
            const provider = meta?.["agentProvider"];
            if (typeof provider === "string" && provider.length > 0) {
                // Agent definitions may store an alias (e.g. "kimi-cli") rather
                // than the canonical provider key ProviderLogo matches on.
                return buildAgentPaneIcon(resolveProviderAlias(provider));
            }
            // Quick-launch path (`launchAgent`) writes `agentId` as the
            // provider key — show the brand logo for that path too.
            const agentId = meta?.["agentId"];
            if (typeof agentId === "string" && PROVIDERS[agentId]) {
                return buildAgentPaneIcon(agentId);
            }
            const icon = meta?.["agentIcon"];
            if (typeof icon === "string" && icon.length > 0) return icon;
            return "sparkles";
        };
        this.viewName = () => {
            const meta = this.blockAtom()?.meta;
            const name = meta?.["agentName"];
            if (typeof name === "string" && name.length > 0) return name;
            return "Agent";
        };
        this.viewText = (): HeaderElem[] => {
            const elems: HeaderElem[] = [];

            // Per-turn live mini-summary — prefers the Haiku-derived
            // term:ambient_summary (useAgentActivitySummary.ts), falling back to
            // the free CLI-emitted term:osc_title (useBlockActivity.ts). Persists
            // across turns; flashes briefly when a new summary lands.
            const activity = readActivitySummary(this.blockAtom()?.meta);
            if (activity && activity.length > 0) {
                elems.push({
                    elemtype: "text",
                    text: activity,
                    className: this._activityFlash() ? "term-activity term-activity--flash" : "term-activity",
                });
            }

            return elems;
        };
        this.noPadding = () => true;
        this.setViewName = async (name: string) => {
            if (!name.trim()) return;
            const oref = WOS.makeORef("block", this.blockId);
            await RpcApi.SetMetaCommand(TabRpcClient, { oref, meta: { agentName: name.trim() } });
        };

        // Pane-frame header button: a single "Stash" (backpack) icon
        // when an agent is loaded — opens the unified tabbed modal
        // (Accounts + Memory). Hidden when no agent is loaded (picker
        // screen). Always shown when agentId exists: quick-launch panes
        // (where agentId is a provider key, not a definition UUID) still
        // get the button — the Accounts tab renders its own empty state
        // for panes without a loadable AgentDefinition, and the Memory
        // tab needs no definition.
        this.endIconButtons = () => {
            const meta = this.blockAtom()?.meta;
            const agentId = meta?.["agentId"];
            // No Stash button on a history-reader tab: _openAgentStashModal
            // is wired up only by AgentPresentationView (the live view),
            // which never mounts for a history tab (AgentHistoryTabView
            // mounts instead per SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md
            // §3.1) — without this the button would render (agentId is
            // copied onto the history block's own meta) but silently do
            // nothing on click. codex P2 on PR #2539.
            if (!agentId || meta?.[HISTORY_TAB_FOR_META_KEY]) return [];
            return [
                {
                    elemtype: "iconbutton",
                    // Deliberately NOT the "vault" icon the global Armory
                    // pane uses — this opens AgentStashModal, the per-agent
                    // analogue of Armory, and a distinct name/icon is the
                    // whole point of the rename (previously shared "vault"
                    // "for parity," which was exactly the confusion between
                    // the two surfaces this fixes). See
                    // docs/reports/REPORT_ARMORY_STASH_NAMING_2026_07_27.md.
                    icon: "backpack",
                    title: "Stash",
                    click: () => { this._openAgentStashModal?.(); },
                },
            ];
        };
    }

    /**
     * Clear the agent-identity meta keys so AgentViewWrapper falls back
     * to the picker. Called from the pane-frame back button and from
     * useAgentCommands.back (which delegates here).
     */
    backToPicker = async (): Promise<void> => {
        const oref = WOS.makeORef("block", this.blockId);
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: null,
                    agentProvider: null,
                    agentOutputFormat: null,
                    agentName: null,
                    agentIcon: null,
                    agentCliPath: null,
                    agentCliArgs: null,
                    agentBinDir: null,
                    controller: null,
                },
            });
        } catch {
            // fail silently — user can manually switch via widget bar
        }
    };

    /**
     * Launch an agent in presentation view.
     * For Phase 1, agentId maps to a provider ID (claude/codex/gemini/kimi/openclaw/pi).
     * Sets block metadata with CLI config and creates a SubprocessController.
     * The agent CLI is not started until the user sends the first message.
     */
    launchAgent = async (agentId: string): Promise<void> => {
        const provider = PROVIDERS[agentId];
        if (!provider) {
            Logger.error("agent", "Unknown agent", { agentId });
            return;
        }

        // Check Node.js availability for npm-based providers
        const nodejsError = await checkNodejsForProvider(provider.id);
        if (nodejsError) {
            this.nodejsError = nodejsError;
            Logger.error("agent", "Node.js not available", { agentId, error: nodejsError });
            return;
        }

        const version = getApi().getAboutModalDetails().version;
        const cliDir = resolveCliDir(version, provider.id);
        const cliBin = `${cliDir}/node_modules/.bin/${provider.cliCommand}`;

        Logger.info("agent", `Launching agent ${agentId} (v${version})`, {
            agentId,
            launchArgs: provider.launchArgs,
            outputFormat: provider.styledOutputFormat,
        });

        const oref = WOS.makeORef("block", this.blockId);
        const blockId = this.blockId;

        // Build CLI args: use persistent args if available, otherwise standard launch args
        const isPersistent = provider.controllerType === "persistent";
        const cliArgs = isPersistent && provider.persistentLaunchArgs
            ? [...provider.persistentLaunchArgs]
            : [...provider.launchArgs];

        // Build env vars: unset nested-session guards by setting them empty
        const envVars: Record<string, string> = {};
        if (provider.unsetEnv) {
            for (const key of provider.unsetEnv) {
                envVars[key] = "";
            }
        }

        // Provider auth isolation (skip if provider has no isolated auth dir configured)
        if (provider.authConfigDirEnvVar) {
            const authDir = await getApi().ensureAuthDir(provider.id);
            envVars[provider.authConfigDirEnvVar] = authDir;
        }
        if (provider.authExtraEnv) {
            Object.assign(envVars, provider.authExtraEnv);
        }
        // Only set exit delay for subprocess mode — persistent processes must stay alive
        if (provider.controllerType !== "persistent") {
            envVars["CLAUDE_CODE_EXIT_AFTER_STOP_DELAY"] = "30000";
        }

        try {
            // Store CLI config in block metadata for the backend to read on AgentInput
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: agentId,
                    agentOutputFormat: provider.styledOutputFormat,
                    controller: isPersistent ? "persistent" : "subprocess",
                    cmd: cliBin,
                    "cmd:args": cliArgs,
                    "cmd:env": envVars,
                    "agent:resume_flag": provider.resumeFlag ?? "",
                    "agent:resume_strategy": provider.resumeStrategy ?? (provider.resumeFlag ? "flag" : "none"),
                    "agent:session_id_field": provider.sessionIdField,
                },
            });

            // Create SubprocessController (no-op start — waits for first message)
            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: atoms.staticTabId(),
                blockid: blockId,
                forcerestart: true,
            });
        } catch (e: any) {
            Logger.error("agent", "Failed to launch agent", { error: String(e) });
        }
    };

    /**
     * Launch a agent in presentation view.
     * Uses the AgentDefinition's provider to look up CLI config.
     * Loads content blobs (soul, agentmd, mcp, env) and writes config files
     * to the working directory via WriteAgentConfigCommand, then creates
     * a SubprocessController ready for user input.
     */
    /**
     * @param targetBlockId In-pane tabs, Phase 4 — when set, launches INTO
     *   that block instead of `this.blockId`. Used for the fork-tab-strip
     *   `+` action, which spawns the fork into a freshly-created,
     *   not-yet-placed block (see `pane.open`'s `skip_placement`) rather
     *   than reconfiguring the current pane's own block. Every other
     *   caller omits this and gets today's behavior unchanged.
     */
    /**
     * @returns Whether the launch actually succeeded. This method never
     *   THROWS (every failure path is caught and logged internally, by
     *   design — most callers fire-and-forget and don't want a launch
     *   failure to crash their UI), but callers that need to react to
     *   failure (e.g. the fork-tab-strip "+" action, which must not push a
     *   permanently-broken tab onto the pane's stack) can check the
     *   returned boolean instead of relying on a rejected promise.
     */
    launchAgentDefinition = async (
        agent: AgentDefinition,
        overrides?: LaunchOverrides,
        targetBlockId?: string,
    ): Promise<boolean> => {
        // See resolveEffectiveLaunchProvider's own doc comment
        // (agent-launch-env.ts) for why this must resolve through the
        // agent's bound bundle rather than trusting `agent.provider`
        // directly.
        const effectiveProvider = await resolveEffectiveLaunchProvider(agent);

        const provider = PROVIDERS[effectiveProvider] ?? PROVIDERS[resolveProviderAlias(effectiveProvider)];
        if (!provider) {
            Logger.error("agent", "Unknown provider in agent definition", { agentId: agent.id, provider: effectiveProvider });
            return false;
        }

        // Check Node.js availability for npm-based providers
        const nodejsError = await checkNodejsForProvider(provider.id);
        if (nodejsError) {
            this.nodejsError = nodejsError;
            Logger.error("agent", "Node.js not available for agent definition", { agentId: agent.id, error: nodejsError });
            return false;
        }

        const version = getApi().getAboutModalDetails().version;
        const cliDir = resolveCliDir(version, provider.id);
        const cliBin = `${cliDir}/node_modules/.bin/${provider.cliCommand}`;

        Logger.info("agent", `Launching agent definition ${agent.name} (${effectiveProvider})`, {
            agentId: agent.id,
            provider: effectiveProvider,
        });

        // Load all content for this agent
        let contents: AgentContent[] = [];
        try {
            contents = await RpcApi.GetAllAgentContentCommand(TabRpcClient, { agent_id: agent.id }) ?? [];
        } catch (e: any) {
            Logger.error("agent", "Failed to load agent content", { error: String(e) });
        }
        const contentMap: Record<string, string> = {};
        for (const c of contents) {
            contentMap[c.content_type] = c.content;
        }

        // Load skills for this agent (lazy-loading: only names/descriptions injected)
        let skills: AgentSkill[] = [];
        try {
            skills = await RpcApi.ListAgentSkillsCommand(TabRpcClient, { agent_id: agent.id }) ?? [];
        } catch (e: any) {
            Logger.error("agent", "Failed to load agent skills", { error: String(e) });
        }

        // Definition slug: stable across launches of this definition.
        // Used for identity-scoped paths (GH_CONFIG_DIR, git author
        // email) so credentials and identity persist even when the
        // user relaunches the same definition with different instance
        // names. See SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md §1.
        const slug = agent.slug || agent.name.toLowerCase().replace(/[^a-z0-9-_]/g, "-");

        // Instance name: overrides.instanceName wins (modal-supplied);
        // falls back to the definition's own name for callers that
        // haven't adopted the modal flow yet (legacy / tests).
        const instanceName = overrides?.instanceName?.trim() || agent.name;

        // Per-launch slug: `<slug>-<YYYYMMDD-HHMMSS>`. Ensures two
        // instances of the same definition never share a working
        // directory even if they're launched with the same name.
        // Definitions without an override retain the legacy
        // slug-only path to avoid churn on existing seeded agents.
        const instanceSlug = overrides?.instanceName
            ? buildInstanceSlug(overrides.instanceName)
            : slug;

        // If the persisted working_directory was seeded with a literal `~/.agentmux/...`
        // path (see `scripts/gen-seed.js`), rewrite its prefix to the host-resolved
        // `agentmuxHome()`. On portable builds that points into the portable data dir;
        // on installed builds it matches the original `~/.agentmux/` verbatim. Without
        // this rewrite the backend rejects the path with "path traversal denied"
        // because `~/` never gets expanded to an absolute path at the OS layer.
        const persisted = agent.working_directory ?? "";
        const workDir = overrides?.instanceName
            ? `${agentmuxHome()}/agents/${instanceSlug}`
            : persisted.startsWith("~/.agentmux/")
                ? `${agentmuxHome()}${persisted.slice("~/.agentmux".length)}`
                : (persisted || `${agentmuxHome()}/agents/${slug}`);

        // Build CLI args: use persistent args if available, otherwise standard launch args
        const isPersistent = provider.controllerType === "persistent";
        const cliArgs = isPersistent && provider.persistentLaunchArgs
            ? [...provider.persistentLaunchArgs]
            : [...provider.launchArgs];
        if (agent.provider_flags) {
            cliArgs.push(...agent.provider_flags.split(/\s+/).filter(Boolean));
        }
        // In-pane tabs, Phase 4 — see LaunchOverrides.forkSession's own doc
        // comment. Review finding: `--fork-session` is Claude Code CLI
        // syntax, validated ONLY for Claude
        // (SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15 §6.4's empirical
        // gate) — gating on "any provider with a resumeFlag" wrongly also
        // matched gemini (`-r`) and muxcode (`--resume`), passing an
        // unsupported flag to CLIs that were never validated to accept it.
        // Every other provider (including those two) silently falls back
        // to "fork = fresh definition, fresh start" — no flag, no error,
        // exactly the graceful fallback §6.4 called for.
        if (overrides?.forkSession && provider.id === "claude") {
            cliArgs.push("--fork-session");
        }

        // Build env vars from provider unsetEnv + agent env content + per-agent isolation
        const envVars: Record<string, string> = {};
        if (provider.unsetEnv) {
            for (const key of provider.unsetEnv) {
                envVars[key] = "";
            }
        }
        if (contentMap["env"]) {
            for (const line of contentMap["env"].split("\n")) {
                const trimmed = line.trim();
                if (!trimmed || trimmed.startsWith("#")) continue;
                const eqIdx = trimmed.indexOf("=");
                if (eqIdx < 1) continue;
                const key = trimmed.substring(0, eqIdx);
                const val = trimmed.substring(eqIdx + 1);
                if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(key)) continue;
                envVars[key] = val;
            }
        }

        // Per-agent GitHub config isolation — keyed by the stable
        // slug so renaming doesn't orphan ~/.agentmux/config/gh-<old>.
        envVars["GH_CONFIG_DIR"] = `${agentmuxHome()}/config/gh-${slug}`;

        // AGENTMUX_AGENT_ID: stable role slug for muxbus routing.
        // Keyed on by send_message/inject_terminal — must not change
        // across respawns of the same definition. Display name is in
        // AGENTMUX_AGENT_DISPLAY.
        envVars["AGENTMUX_AGENT_ID"] = slug;
        // Stable definition slug — survives renames and re-launches.
        // Downstream code reads this when it needs the rename-stable
        // form (e.g. session-id lookup).
        envVars["AGENTMUX_AGENT_SLUG"] = slug;
        // Per-instance slug: the working-directory suffix. Lets
        // downstream code find the instance's scratch dir without
        // re-parsing paths.
        envVars["AGENTMUX_INSTANCE_SLUG"] = instanceSlug;
        // Explicit display alias. Human-readable, unaffected by
        // slug vs id collapsing in future migrations.
        envVars["AGENTMUX_AGENT_DISPLAY"] = instanceName;

        // Git identity — prevents "Please tell me who you are" errors when
        // the host machine has no global git config. Uses the instance
        // name for author/committer and the stable definition slug for
        // the placeholder email so commits attribute to "this run" while
        // still grouping by definition identity.
        envVars["GIT_AUTHOR_NAME"]     = instanceName;
        envVars["GIT_AUTHOR_EMAIL"]    = `${slug}@agents.local`;
        envVars["GIT_COMMITTER_NAME"]  = instanceName;
        envVars["GIT_COMMITTER_EMAIL"] = `${slug}@agents.local`;
        // GIT_CONFIG_GLOBAL is intentionally not set: we use the 4 identity
        // env vars above which git always honours, avoiding any path-handling edge cases.

        // Provider auth: the default lives in the account-wide, version- and
        // channel-independent shared dir (~/.agentmux/shared/providers/<provider>/),
        // resolved by ensureAuthDir → ensure_auth_dir; one login is shared across
        // every instance / channel / version. Skip the env var only for providers
        // with no isolated auth dir configured.
        if (provider.authConfigDirEnvVar) {
            const authDir = await getApi().ensureAuthDir(provider.id);
            envVars[provider.authConfigDirEnvVar] = authDir;
        }
        if (provider.authExtraEnv) {
            Object.assign(envVars, provider.authExtraEnv);
        }
        // Model vendor override (harness vs. model-vendor decoupling) — e.g.
        // ANTHROPIC_BASE_URL for a claude-provider agent pointed at a proxy/
        // Bedrock/OpenRouter instead of Anthropic's default endpoint. Mirrors
        // agent_open.rs's resolve_vendor_env_override: only injected when
        // both the agent has a non-empty override AND the provider declares
        // support for it (should already be guaranteed by agent.define's
        // validation, but this doesn't trust that as the only gate).
        const vendorOverride = resolveVendorEnvOverride(agent.model_vendor_base_url, provider.baseUrlEnvVar);
        if (vendorOverride) {
            const [envVar, value] = vendorOverride;
            envVars[envVar] = value;
        }
        // Only set exit delay for subprocess mode — persistent processes must stay alive
        if (provider.controllerType !== "persistent") {
            envVars["CLAUDE_CODE_EXIT_AFTER_STOP_DELAY"] = "30000";
        }

        // Build config files to write via backend RPC. Pass the
        // resolved instance name so the injected agentmux MCP server
        // advertises the same identity the shell / git / pane title
        // already use — otherwise inter-agent messaging routes on
        // the definition name while everything else advertises the
        // instance name (caught by codex on PR #504).
        const configFiles = buildConfigFiles(contentMap, skills, agent, instanceName);

        const blockId = targetBlockId ?? this.blockId;
        const oref = WOS.makeORef("block", blockId);
        try {
            // Whether the work_dir was constructed by us (and is thus
            // eligible for `<base>-N` collision suffixing) or was
            // pulled verbatim from the agent definition's
            // `working_directory` field (which we must NEVER rewrite,
            // even if it happens to live under ~/.agentmux/ — a user
            // can legitimately set `agent.working_directory` to
            // `~/.agentmux/my-project` and expect that exact dir).
            //
            // Auto cases:
            //   - overrides.instanceName given (modal launch chose a
            //     fresh per-launch slug)
            //   - working_directory is empty (we filled in the legacy
            //     ~/.agentmux/agents/<slug> default ourselves)
            // Any other persisted value is user-specified and stays
            // verbatim through allocation.
            //
            // v8 continuation path: `overrides.workDirOverride` short-
            // circuits the auto-allocate logic entirely. The launch
            // modal's "Continue agent" dropdown sets this from the
            // prior instance's `working_directory`. We pass
            // auto_allocate: false so WriteAgentConfigCommand reuses
            // the existing directory (overwriting config files is
            // intentional — bundles can change between sessions).
            const continueWorkDir = overrides?.workDirOverride?.trim() ?? "";
            const isContinue = continueWorkDir.length > 0;
            const autoAllocate =
                !isContinue &&
                (Boolean(overrides?.instanceName) ||
                    (agent.working_directory ?? "").trim() === "");
            const writeWorkDir = isContinue ? continueWorkDir : workDir;

            // Allocate the workdir + write config files BEFORE we set
            // cmd:cwd, so the meta records the actual collision-
            // resolved path the controller will spawn the CLI in.
            // Always call WriteAgentConfigCommand (even with no files
            // to write) when auto-allocate so we get the atomic
            // `mkdir` reservation. Returns the final path.
            const writeResult = await RpcApi.WriteAgentConfigCommand(TabRpcClient, {
                working_dir: writeWorkDir,
                files: configFiles,
                auto_allocate: autoAllocate,
            });
            const finalWorkDir = writeResult?.working_dir || writeWorkDir;

            // Store CLI config in block metadata using the (possibly
            // collision-resolved) finalWorkDir.
            //
            // Two-tier picker reattach (2026-05-24): the new block's
            // `agent:sessionid` MUST mirror the launch intent
            // exactly. SetMetaCommand merges meta (it doesn't
            // replace), so an empty/omitted key on a REUSED block
            // would leave any prior block's `agent:sessionid` in
            // place — and the backend would then append
            // `--resume <stale>` on a greenfield launch, resuming
            // the wrong conversation (codex P1 round 2 on PR
            // #1018).
            //
            // Set the key UNCONDITIONALLY:
            //   - continueSessionId non-empty → set to that id;
            //     spawn_turn hydrates inner.session_id and appends
            //     `--resume <sid>` on the FIRST turn.
            //   - continueSessionId empty (greenfield) → set to ""
            //     to clear any stale residue. The backend
            //     (`meta_get_string` with default "") already
            //     treats "" as "no value" and sets
            //     SubprocessSpawnConfig::session_id = None, so no
            //     resume flag is appended.
            //
            // The captured-id-wins invariant in the controller
            // ensures any session id the CLI later emits on stdout
            // overrides whatever value lands here on subsequent
            // turns.
            const continueSid = overrides?.continueSessionId?.trim() ?? "";

            // Per-agent zoom persistence (SPEC_AGENT_ZOOM_PERSISTENCE_2026_06_22.md):
            // seed term:zoom from the agent's saved ui:zoom (already loaded
            // into contentMap above) so reopening the same agent restores
            // its zoom instead of resetting to 1.0. This path never went
            // through the backend's own seed (agent_open.rs::
            // register_agent_open) — see this file's own launch pipeline.
            // Set UNCONDITIONALLY (parseSeedZoom's null, not an omitted
            // key) — see that function's doc comment for why.
            const zoomMeta: Record<string, unknown> = {
                "term:zoom": parseSeedZoom(contentMap["ui:zoom"]),
            };

            // Per-agent color (SPEC_AGENT_COLOR_2026_08_08.md): seed the
            // frame border colors from the agent's stored ui:color —
            // full-strength on the focused border
            // (frame:activebordercolor), dimmed on the unfocused one
            // (frame:bordercolor) so the color is visible either way while
            // focus stays distinguishable by brightness. Assign-if-missing
            // write-through covers an agent that predates migration m0020
            // or was created before this path existed.
            const rawColor = contentMap["ui:color"]?.trim();
            let agentColor: string;
            if (isValidAgentColor(rawColor)) {
                agentColor = rawColor;
            } else {
                agentColor = pickAgentColor(agent.id);
                RpcApi.SetAgentContentCommand(TabRpcClient, {
                    agent_id: agent.id,
                    content_type: "ui:color",
                    content: agentColor,
                }).catch((e: any) => {
                    Logger.warn("agent", "Failed to persist assigned agent color", { error: String(e) });
                });
            }

            // Seed the initial runtime config (permission mode / model /
            // effort — see resolveInitialRuntimeConfig's own doc comment
            // for why launchAgentDefinition never set this key at all
            // before now).
            const runtimeConfig = resolveInitialRuntimeConfig(overrides?.model, provider.models);
            const meta: Record<string, unknown> = {
                agentId: agent.id,
                agentProvider: effectiveProvider,
                agentOutputFormat: provider.styledOutputFormat,
                agentName: instanceName,
                agentIcon: agent.icon,
                agentMode: overrides?.agentType ?? agent.agent_type ?? "host",
                ...(overrides?.containerImage || agent.container_image ? { "agent:container_image": overrides?.containerImage || agent.container_image } : {}),
                controller: isPersistent ? "persistent" : "subprocess",
                cmd: cliBin,
                "cmd:args": cliArgs,
                "cmd:cwd": finalWorkDir,
                "cmd:env": envVars,
                "agent:resume_flag": provider.resumeFlag ?? "",
                "agent:resume_strategy": provider.resumeStrategy ?? (provider.resumeFlag ? "flag" : "none"),
                "agent:session_id_field": provider.sessionIdField,
                "agent:sessionid": continueSid,
                "agent:runtime": runtimeConfig,
                ...zoomMeta,
                "frame:activebordercolor": agentColor,
                "frame:bordercolor": dimAgentColor(agentColor),
            };
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta,
            });

            // Create SubprocessController (no-op start — waits for first message)
            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: atoms.staticTabId(),
                blockid: blockId,
                forcerestart: true,
            });

            // Record this launch as an `AgentInstance` row in the DB so the
            // backend can track which pane is running which definition,
            // surface concurrent launches, and (later) carry GitHub work
            // context. Best-effort — a failure here doesn't abort the
            // launch (the agent already started). Stash the instance id
            // into block meta so downstream code (status updates,
            // bus targeting, lineage) can reference it. See
            // specs/archive/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md §Phase 5.
            try {
                const inst = await RpcApi.CreateAgentInstanceCommand(TabRpcClient, {
                    definition_id: agent.id,
                    block_id: blockId,
                    // PR-F.3: launch modal carries the user's bundle
                    // picks. Empty / "blank" no longer buys ambient-creds
                    // fallback — the backend resolver's layer-3 gate now
                    // requires a real bound account for any oauth-class
                    // provider regardless of identity_id's value (#2463
                    // Finding 2). Issue #1624 PR-C Part B — `accountId`
                    // replaces the old bundle-id `identityId`; this column
                    // keeps its name (`identity_id`) since it's a legacy
                    // `db_agent_instances` field, not part of the new
                    // direct-link system.
                    identity_id: overrides?.accountId,
                    memory_id: overrides?.memoryId,
                    // v8: named-agent continuation. The instance name
                    // is the AGENTMUX_AGENT_ID the user picked in the
                    // modal; finalWorkDir is the path that
                    // WriteAgentConfigCommand resolved (after slug
                    // collision suffixing). Both are persisted so the
                    // launch modal's "Continue agent" dropdown can
                    // surface this instance later. See
                    // SPEC_NAMED_AGENT_CONTINUATION_2026_05_12.md.
                    instance_name: instanceName,
                    working_directory: finalWorkDir,
                    // v8 continuation: chain lineage to the prior row
                    // so the dropdown query can collapse continuations
                    // (filter parent_instance_id = '' surfaces only
                    // the "root" of each chain).
                    parent_instance_id: overrides?.continueOfInstanceId,
                });
                await RpcApi.SetMetaCommand(TabRpcClient, {
                    oref,
                    meta: { agentInstanceId: inst.id },
                });
            } catch (e: any) {
                Logger.warn(
                    "agent",
                    `agent instance row create failed: ${e?.message ?? String(e)}`,
                );
            }

            // Write-through: link this agent definition directly to the
            // selected account for its own provider. Issue #1624 PR-C
            // Part B — the launch modal now picks an account directly
            // (no bundle-of-bindings to reconcile against), so this
            // collapses to a single upsert. `agent_identity_link`'s
            // `ON CONFLICT(agent_id, provider) DO UPDATE` (identities.rs)
            // makes one call correct without a diff/unlink pass.
            //
            // Best-effort, same as the instance-row write above: a
            // failure here doesn't abort the launch, since the agent
            // already started. This also covers the accepted migration
            // gap where a pre-issue-#1624-PR-C continuation carries a
            // stale bundle id instead of a real account id — the RPC
            // fails gracefully and just logs a warning. See
            // docs/specs/SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10.md.
            //
            // #2463 Finding 1: defense-in-depth — every known call site
            // already filters its own `accountId` before it reaches
            // `launchAgentDefinition`, but a stale reference reaching this
            // RPC throws `FOREIGN KEY constraint failed` against
            // `db_accounts`, which this try/catch silently swallows.
            // `realAccountIdOrEmpty` cross-checks against a fresh account
            // fetch (not just UUID shape — a legacy identity-bundle id
            // would pass a shape-only check too, codex P1 on this fix's
            // PR; not the synchronous loadAccounts() cache either, which
            // can still be mid-priming — reagentx P2 on the same PR), so a
            // missed/future call site fails safe instead of depending on
            // every caller filtering perfectly.
            try {
                const accountId = realAccountIdOrEmpty(
                    overrides?.accountId ?? "",
                    (await refreshAccountCache()).map((a) => a.id),
                );
                if (accountId) {
                    await RpcApi.LinkAgentIdentityCommand(TabRpcClient, {
                        agent_id: agent.id,
                        account_id: accountId,
                        provider: effectiveProvider,
                    });
                }
            } catch (e: any) {
                Logger.warn(
                    "agent",
                    `direct identity link write-through failed: ${e?.message ?? String(e)}`,
                );
            }
            return true;
        } catch (e: any) {
            Logger.error("agent", "Failed to launch agent definition", { error: String(e) });
            return false;
        }
    };

    /**
     * Right-click body context menu — blockframe.tsx splices this at the
     * TOP of the menu, before the shared pane actions + separator (see its
     * body-right-click handler). "Agent History" is a plain top-level item,
     * not wrapped in a `submenu` — the ask is one clickable action (open
     * or focus the tab), not a choice among several; `submenu` is a
     * drop-in extension point here if a future need for several history
     * destinations (session / full / archives) arises. No-op when there's
     * no agent loaded yet (still on the picker).
     *
     * Spec: SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.3.
     */
    getBodyContextMenuItems(): ContextMenuItem[] {
        const meta = this.blockAtom()?.meta;
        const agentId = meta?.["agentId"] as string | undefined;
        // No entry on the history tab itself (nothing to open — it'd just
        // re-focus this same tab) and none before an agent is loaded.
        if (!agentId || meta?.[HISTORY_TAB_FOR_META_KEY]) return [];
        return [
            {
                label: "Agent History",
                click: () => void openOrFocusHistoryTab({ currentBlockId: this.blockId, agentId }),
            },
            { type: "separator" },
        ];
    }

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {}
}

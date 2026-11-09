// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, createSignal, type Accessor } from "solid-js";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import type { PaneVoiceHandle } from "@/app/hook/useVoiceInput";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { atoms, getApi, MOS } from "@/app/store/global";
import { agentModels } from "./agent-models";
import { buildAgentPaneIcon } from "./components/AgentPaneIcon";
import { paneIconInput, samePaneIconInput } from "./pane-icon-input";
import { useAgentDefinitions } from "./components/AgentPicker";
import { getProvider, isAccountListed, PROVIDERS } from "./providers";
import { Logger } from "@/util/logger";
import { translateError } from "@/app/errors/translate";
import { buildInstanceSlug } from "./defaults/instance-slug";
import { archiveThenReturnToPicker, newSessionArchives } from "./start-new-session";
import { leavePaneAgent } from "./leave-pane-agent";
import type { LaunchOverrides } from "./launch-overrides";
import { deriveSlug } from "./agent-slug";
import { checkNodejsForProvider, ensureProviderAuthDir, resolveCliBin, resolveEffectiveLaunchProvider, resolveLaunchRuntime, commitLaunch } from "./agent-launch-env";
import { resolveForkSessionArgs } from "./fork-session-args";
import { HISTORY_TAB_FOR_META_KEY, historyTabLabel } from "./open-history-tab";
import { agentBodyMenuItems } from "./agent-body-menu";
import { requestShutdown } from "./shutdown/shut-down-agent";
import { agentViewControls } from "./agent-view-controls";
import type { ModalLayerRequest } from "@/app/element/modal-layer";
import { quickForkAgent } from "./quick-fork";
import { cancelComposerFocusRequest, focusComposer, requestComposerFocus } from "../../agent-model/composer-focus";
import { isPersistentLaunch, launchController, TERMINAL_PANE_META } from "./launch-args";
import { recallRuntime } from "./remembered-runtime";
import { buildPaneArgs } from "./buildRuntimeArgs";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { markAgentOpen } from "./open-trace";
import { messagesHeaderButtons } from "./messages/messages-state";
import { searchHeaderButtons } from "./search/search-bridge";
import { launchResumesConversation, requestCatchUp } from "./catch-up-request";
import { isContainerAgentMode, isContainerAgentType } from "@/util/runtime-kind";
import {
    allocateLaunchWorkDir,
    buildLaunchEnv,
    buildLaunchMeta,
    createLaunchInstance,
    launchAgentColor,
    launchWorkDir,
    linkLaunchAccount,
    loadAgentContent,
} from "./launch-agent-steps";

/** What the first stage of a launch resolved: the harness, and where its CLI is on the host. */
interface LaunchTarget {
    effectiveProvider: string;
    provider: NonNullable<ReturnType<typeof getProvider>>;
    cliBin: string | undefined;
}

/** What the second stage worked out for the launch to carry. */
interface LaunchPlan {
    contentMap: Awaited<ReturnType<typeof loadAgentContent>>;
    instanceName: string;
    agentMode: string;
    workDir: string;
    paneFlags: string;
    runtimeConfig: ReturnType<typeof resolveLaunchRuntime>["runtimeConfig"];
    cliArgs: string[];
    forkSessionArgs: ReturnType<typeof resolveForkSessionArgs>;
    envVars: Awaited<ReturnType<typeof buildLaunchEnv>>;
}


/** The agent's state behind its native pane tab (`agentPaneTabManifest`,
 *  agent-manifest.tsx). */
export class AgentViewModel {
    viewType = "agent";
    blockId: string;
    /** This block, as its components read it (only its meta is ever read);
     *  built from the host context's meta. */
    blockAtom: Accessor<Block | undefined>;

    viewIcon: () => string | IconButtonDecl;
    viewName: () => string;
    setViewName: (name: string) => Promise<void>;
    viewText: () => string | HeaderElem[];
    setProgressBarMount: (el: HTMLDivElement | null) => void;
    /** NOT part of the shared `ViewModel` contract — `AgentBlockContent`
     *  reads this directly off its own concrete `AgentViewModel` instance
     *  to know where to portal the marching-ants progress bar. Populated
     *  externally by the shared pane chrome's own progress-bar slot
     *  (PaneChrome.tsx, via `setProgressBarMount`) while this model is the
     *  pane's active one — `null` otherwise. */
    progressBarMount: () => HTMLDivElement | null;
    /** NOT part of the shared `ViewModel` contract. ReAgent P2 on
     *  SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md's PR: owned here
     *  (one `useAgentDefinitions()` subscription per ViewModel instance,
     *  matching the granularity content was already reconstructed at) so
     *  both `AgentBlockContent` (via `model.agentDefinitions`) and
     *  the agent pane chrome (via `agentModels.get(activeBlockId)`)
     *  read the SAME list instead of each independently calling the hook —
     *  the exact redundant-RPC-plus-subscription pattern this file's own
     *  header comment (reagent P2 on PR #2488) already warns against. */
    agentDefinitions: () => AgentDefinition[];
    endIconButtons: () => (IconButtonDecl | ToggleIconButtonDecl)[];
    nodejsError: string | null = null;
    /** Why the last launch aborted, for the picker to show (identity M4b-3). */
    launchError: string | null = null;

    // Opens a modal in the pane's own layer, for the right-click menu's Seat…
    // and Bundles…; wired by AgentPresentationView on mount.
    _openModal: ((req: ModalLayerRequest) => void) | null = null;

    // Voice-input target ref. AgentFooter populates this on mount with a
    // textarea-backed handle (and clears it on unmount). The exposed
    // `voiceHandle` accessor below delegates to whatever's current —
    // before AgentFooter mounts (or after it unmounts) it's a no-op.
    voiceTargetRef: { current: PaneVoiceHandle | null } = { current: null };

    voiceHandle = (): PaneVoiceHandle => ({
        appendFinal: (text: string) => this.voiceTargetRef.current?.appendFinal(text),
        setInterim: (text: string) => this.voiceTargetRef.current?.setInterim(text),
    });

    // Same shape as voiceTargetRef above: AgentFooter populates this on
    // mount with the live composer textarea and clears it on unmount.
    // giveFocus() below is a no-op until then, which is correct — there's
    // nothing to focus yet.
    focusTargetRef: { current: HTMLTextAreaElement | null } = { current: null };

    private unregisterModel: () => void;
    private ctx: PaneTabHostContext;

    // A native pane tab (Pane Tab contract Phase 2c): built by `create(ctx)`
    // (agent-manifest.tsx) in its own reactive root. Its own block's meta
    // comes from, and goes to, the host context; `launchAgentDefinition` can
    // target ANOTHER block, so that path still writes by block id.
    constructor(ctx: PaneTabHostContext) {
        const blockId = ctx.blockId;
        this.ctx = ctx;
        this.blockId = blockId;
        this.blockAtom = createMemo(() => ({ oid: blockId, meta: ctx.meta() }) as Block);
        const [progressBarMountSig, setProgressBarMountSig] = createSignal<HTMLDivElement | null>(null);
        this.progressBarMount = progressBarMountSig;
        this.setProgressBarMount = (el: HTMLDivElement | null) => setProgressBarMountSig(el);
        this.agentDefinitions = useAgentDefinitions()[0];

        // Drive the pane's title from block meta — launching an agent sets
        // `agentName` / `agentIcon` and the frame title automatically picks
        // them up via the blockAtom subscription. Before this, the title
        // was the literal string "Agent" regardless of which agent ran.
        // See SPEC_AGENT_PANE_FOLLOWUPS item #8.
        // Compared by value: a meta write that leaves the harness and the model alone builds no new icon, and a
        // model switched in place changes the badge.
        const iconInput = createMemo(
            () => paneIconInput(this.blockAtom()?.meta, this.agentDefinitions()),
            null,
            { equals: samePaneIconInput }
        );
        this.viewIcon = (): string | IconButtonDecl => {
            // The harness (main icon) and the model answering (badge), from the pane's meta. An agent definition
            // may store an alias (e.g. "kimi-cli") for its provider, and the quick-launch path writes `agentId`
            // as the provider key: paneIconInput resolves both.
            const shown = iconInput();
            if (shown) return buildAgentPaneIcon(shown);
            const meta = this.blockAtom()?.meta;
            const icon = meta?.["agentIcon"];
            if (typeof icon === "string" && icon.length > 0) return icon;
            return "sparkles";
        };
        this.viewName = () => {
            const meta = this.blockAtom()?.meta;
            const name = meta?.["agentName"];
            if (meta?.[HISTORY_TAB_FOR_META_KEY]) return historyTabLabel(name);
            if (typeof name === "string" && name.length > 0) return name;
            // No agent launched into this pane yet (the picker state) — same
            // label a newly-"+"-created tab falls back to (agent-view.tsx's
            // own `?? "Agent"`), so a fresh pane and a fresh tab read
            // identically.
            return "Agent";
        };
        // The per-turn Haiku-derived mini-summary (term:ambient_summary,
        // useAgentActivitySummary.ts) used to render here via readActivitySummary.
        // Removed from the pane header 2026-09-20 (user request, part of the
        // pane-color unification pass) — generation itself is untouched since
        // swarm-model.ts also reads term:ambient_summary for the Swarm view.
        this.viewText = (): HeaderElem[] => [];
        this.setViewName = async (name: string) => {
            if (!name.trim()) return;
            await this.ctx.setMeta({ agentName: name.trim() });
        };

        // The header has no buttons of its own: the per-agent Seat, Bundles and
        // Personal memory entries live in the right-click menu
        // (agent-body-menu.ts), and the account, MCP and skill bindings in
        // Connectors and Memory (SPEC_DROP_THE_AGENT_STASH_2026_10_24.md, D6).
        // The one exception is the find magnifier (search/search-bridge.ts).
        this.endIconButtons = () => [...searchHeaderButtons(blockId), ...messagesHeaderButtons(blockId)];

        // Last, once every field exists: registering notifies readers of
        // this block (the pane chrome's fork tabs), which then read those
        // fields (ReAgent P1 on #3807).
        this.unregisterModel = agentModels.register(blockId, this);
    }

    /**
     * Take this pane's agent out of it, in place, so AgentViewWrapper falls back to the picker: stop its CLI, then
     * clear what the pane kept of it (`leavePaneAgent`). Called from the pane-frame back button and from
     * useAgentCommands.back (which delegates here), and by Start a new session.
     */
    backToPicker = async (): Promise<void> => {
        try {
            await leavePaneAgent(this.blockId, {
                stop: (blockId) => RpcApi.AgentStopCommand(TabRpcClient, { blockid: blockId, force: true }),
                setMeta: (meta) => this.ctx.setMeta(meta),
                warn: (message) => Logger.warn("agent", message),
            });
        } catch {
            // fail silently — user can manually switch via widget bar
        }
    };

    /** Start a fresh conversation for this agent; see `newSessionArchives`. */
    startNewSession = (): Promise<void> =>
        archiveThenReturnToPicker(
            newSessionArchives(
                this.blockId,
                this.blockAtom()?.meta?.["agentId"],
                (blockId) => RpcApi.SessionArchiveCommand(TabRpcClient, { block_id: blockId }),
                (definitionId) => RpcApi.AgentSessionArchiveCommand(TabRpcClient, { definition_id: definitionId })
            ),
            this.backToPicker,
            (e) => Logger.error("agent", "Archiving the current session before a new one failed", { error: String(e) })
        );

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
        const nodejsError = await checkNodejsForProvider(provider);
        if (nodejsError) {
            this.nodejsError = nodejsError;
            Logger.error("agent", "Node.js not available", { agentId, error: nodejsError });
            return;
        }

        const version = getApi().getAboutModalDetails().version;
        // Where the backend actually installed the CLI -- see resolveCliBin.
        let cliBin: string;
        try {
            cliBin = await resolveCliBin(provider, this.blockId, this.blockId, agentId);
        } catch (e) {
            const t = translateError(e);
            this.launchError = `${t.title}: ${t.message}`;
            Logger.error("agent", "Could not resolve the agent CLI", { agentId, error: String(e) });
            return;
        }

        Logger.info("agent", `Launching agent ${agentId} (v${version})`, {
            agentId,
            launchArgs: provider.launchArgs,
            outputFormat: provider.styledOutputFormat,
        });

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
            const authDir = await ensureProviderAuthDir(provider.id);
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
            await this.ctx.setMeta({
                agentId: agentId,
                agentOutputFormat: provider.styledOutputFormat,
                // ACP harnesses (Copilot, OpenClaw, Pi) need srv's AcpController;
                // a container agent keeps the per-turn path into its container
                // (SPEC_ACP_CLIENT_CONFORMANCE_2026_10_07.md).
                controller: launchController(provider, undefined),
                cmd: cliBin,
                "cmd:args": cliArgs,
                "cmd:env": envVars,
                "agent:resume_flag": provider.resumeFlag ?? "",
                "agent:resume_strategy": provider.resumeStrategy ?? (provider.resumeFlag ? "flag" : "none"),
                "agent:session_id_field": provider.sessionIdField,
                "agent:prompt_arg_flag": provider.promptArgFlag ?? "",
                ...(provider.controllerType === "terminal" ? TERMINAL_PANE_META : {}),
            });

            // Create SubprocessController (no-op start — waits for first message).
            // A terminal-tier CLI is started by its term view instead (see
            // launchAgentDefinition).
            if (provider.controllerType !== "terminal") {
                await RpcApi.ControllerResyncCommand(TabRpcClient, {
                    tabid: atoms.staticTabId(),
                    blockid: blockId,
                    forcerestart: true,
                });
            }
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
     * @param targetBlockId When set, launches INTO that block instead of
     *   `this.blockId` — used by `quick-fork.ts`'s `quickForkAgent`, which
     *   spawns the fork into a freshly-created block that `pane.open
     *   { stack_onto_block_id }` placed in THIS SAME pane's own block-stack
     *   (created and placed in one backend step), rather than reconfiguring
     *   the current pane's own block. Every other caller omits this and gets today's behavior
     *   unchanged.
     * @param targetTabId The tab id to pass to `ControllerResyncCommand`
     *   instead of `atoms.staticTabId()` (fixed at window bootstrap, not
     *   necessarily the tab this launch is actually happening in). Required
     *   whenever the caller's own operation spans an `await` between
     *   resolving "which tab am I in" and this call — `quickForkAgent`
     *   captures the active tab id synchronously before its own several
     *   RPCs, since the user could switch window tabs mid-flight otherwise,
     *   silently registering the new block's controller under the wrong tab
     *   (Codex's review of PR #2746, one of two P1s from the same root
     *   cause). Every other caller omits this and gets today's
     *   `atoms.staticTabId()` behavior unchanged.
     */
    /**
     * @returns Whether the launch actually succeeded. This method never
     *   THROWS (every failure path is caught and logged internally, by
     *   design — most callers fire-and-forget and don't want a launch
     *   failure to crash their UI), but callers that need to react to
     *   failure (e.g. `quickForkAgent`, which must not leave a
     *   permanently-broken tab on the pane's stack) can check the
     *   returned boolean instead of relying on a rejected promise.
     */
    launchAgentDefinition = async (
        agent: AgentDefinition,
        overrides?: LaunchOverrides,
        targetBlockId?: string,
        targetTabId?: string,
    ): Promise<boolean> => {
        const target = await this.resolveLaunchTarget(agent, overrides, targetBlockId);
        if (target == null) return false;
        const plan = await this.planLaunch(agent, overrides, target);
        if (plan == null) return false;
        return this.startLaunch(agent, overrides, target, plan, targetBlockId, targetTabId);
    };

    /**
     * The first stage of a launch: the provider the agent runs on, whether it runs in a container,
     * and (on the host) Node.js and the CLI. Null when the launch cannot go on, with the reason set
     * on the model.
     */
    private async resolveLaunchTarget(
        agent: AgentDefinition,
        overrides: LaunchOverrides | undefined,
        targetBlockId: string | undefined,
    ): Promise<LaunchTarget | null> {
        // The shared resolver (agent-launch-env.ts), the one the server's spawn and credential gate
        // mirror: the agent's own provider, or its bundle's only when it has none.
        const effectiveProvider = await resolveEffectiveLaunchProvider(agent);
        const provider = getProvider(effectiveProvider);
        if (!provider) {
            Logger.error("agent", "Unknown provider in agent definition", { agentId: agent.id, provider: effectiveProvider });
            return null;
        }

        // A container agent's CLI is installed inside its container on the first message, so launching
        // it needs no Node.js and no CLI on this computer (SPEC_CONTAINER_AGENTS_AT_PARITY_2026_10_27.md, P10).
        const runsInContainer = isContainerAgentType(overrides?.agentType ?? agent.agent_type);
        const nodejsError = runsInContainer ? null : await checkNodejsForProvider(provider);
        if (nodejsError) {
            this.nodejsError = nodejsError;
            Logger.error("agent", "Node.js not available for agent definition", { agentId: agent.id, error: nodejsError });
            return null;
        }

        // Where the backend actually installed the CLI -- see resolveCliBin.
        let cliBin: string | undefined;
        if (!runsInContainer) {
            try {
                cliBin = await resolveCliBin(provider, this.blockId, targetBlockId ?? this.blockId, agent.id);
            } catch (e) {
                const t = translateError(e);
                this.launchError = `${t.title}: ${t.message}`;
                Logger.error("agent", "Could not resolve the agent CLI for agent definition", { agentId: agent.id, error: String(e) });
                return null;
            }
        }
        return { effectiveProvider, provider, cliBin };
    }

    /**
     * The second stage: the names, the folder, the mode, the runtime and the arguments and
     * environment the launch will carry. Null when the harness cannot run in the mode asked for.
     */
    private async planLaunch(
        agent: AgentDefinition,
        overrides: LaunchOverrides | undefined,
        target: LaunchTarget,
    ): Promise<LaunchPlan | null> {
        const { effectiveProvider, provider } = target;
        Logger.info("agent", `Launching agent definition ${agent.name} (${effectiveProvider})`, {
            agentId: agent.id,
            provider: effectiveProvider,
        });
        const contentMap = await loadAgentContent(agent.id);

        // The definition slug is stable across launches (identity-scoped paths such as GH_CONFIG_DIR
        // and the git author email use it); the instance name comes from the launch modal, else the
        // definition's own name; a per-launch instance slug keeps two instances out of one folder.
        // SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md §1.
        const slug = agent.slug || deriveSlug(agent.name);
        const instanceName = overrides?.instanceName?.trim() || agent.name;
        const instanceSlug = overrides?.instanceName ? buildInstanceSlug(overrides.instanceName) : slug;
        const workDir = launchWorkDir(agent, overrides, slug, instanceSlug);

        // A CONTAINER agent is never persistent, whatever the provider says: it runs one `docker exec`
        // per turn, so it must take `launchArgs`, not `persistentLaunchArgs` (which carry
        // `--input-format stream-json`; the container's CLI would then expect JSON envelopes on stdin,
        // while the container spawn writes raw text, and the exec dies on the first line). srv's
        // `agent_open.rs` derives `is_persistent` after the same override.
        const agentMode = overrides?.agentType ?? agent.agent_type ?? "host";
        if ((provider.controllerType === "terminal" || provider.controllerType === "msp") && isContainerAgentMode(agentMode)) {
            this.launchError = `${provider.displayName} runs on this computer, not in a container.`;
            return null;
        }
        const isPersistent = isPersistentLaunch(provider, agentMode);
        // The runtime config the strip will show, resolved BEFORE the args so both come from one
        // value: `cmd:args` must already carry `--model`/`--effort`, since a running persistent process
        // never re-reads it (docs/retro/RETRO_RESUMED_AGENT_SPAWNS_WITHOUT_RUNTIME_FLAGS_2026_09_30.md).
        // What the agent remembers from its last pick sits between a fork's carry-over and the
        // definition's flags (see resolveLaunchRuntime).
        const remembered = await recallRuntime(
            agent.id,
            provider.models?.map((m) => m.value),
        );
        const { runtimeConfig, paneFlags } = resolveLaunchRuntime(
            overrides?.model,
            provider,
            agent.provider_flags,
            remembered,
            overrides?.carryOverRuntime,
            provider.id !== "codex" || isAccountListed("codex"),
        );
        const cliArgs = buildPaneArgs(provider, agentMode, runtimeConfig, paneFlags);
        // `--fork-session` is Claude Code CLI syntax; every other provider falls back to a true fresh
        // start (see fork-session-args.ts and LaunchOverrides.forkSession).
        const forkSessionArgs = resolveForkSessionArgs(overrides, provider.id);
        if (forkSessionArgs.appendForkFlag) {
            cliArgs.push("--fork-session");
        }
        const envVars = await buildLaunchEnv({ agent, provider, slug, instanceSlug, instanceName, contentMap, isPersistent });
        return { contentMap, instanceName, agentMode, workDir, paneFlags, runtimeConfig, cliArgs, forkSessionArgs, envVars };
    }

    /**
     * The last stage: write the block's meta, record the launch, start the controller, link the
     * account and, for a resume, ask the agent to catch up. Never throws; says whether it started.
     */
    private async startLaunch(
        agent: AgentDefinition,
        overrides: LaunchOverrides | undefined,
        target: LaunchTarget,
        plan: LaunchPlan,
        targetBlockId: string | undefined,
        targetTabId: string | undefined,
    ): Promise<boolean> {
        const { effectiveProvider, provider, cliBin } = target;
        const { contentMap, instanceName, agentMode, workDir, paneFlags, runtimeConfig, cliArgs, forkSessionArgs, envVars } = plan;
        const blockId = targetBlockId ?? this.blockId;
        const oref = MOS.makeORef("block", blockId);
        try {
            const finalWorkDir = await allocateLaunchWorkDir({ agent, overrides, workDir, instanceName, agentMode });
            markAgentOpen(blockId, "config");
            const agentColor = launchAgentColor(agent, contentMap);
            const meta = buildLaunchMeta({
                agent,
                effectiveProvider,
                provider,
                instanceName,
                agentMode,
                overrides,
                paneFlags,
                cliBin,
                cliArgs,
                finalWorkDir,
                envVars,
                runtimeConfig,
                continueSid: forkSessionArgs.continueSessionId,
                contentMap,
                agentColor,
            });
            // The composer mounts as soon as the meta write below lands; ask it to take focus so the
            // user can type right away (SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §3).
            if (provider.controllerType !== "terminal") requestComposerFocus(blockId);
            // Record the launch row, THEN one SetMeta carrying the block meta and its stamp, THEN
            // resync, which creates the controller (see `commitLaunch`).
            const committed = await commitLaunch({
                isTemplate: agent.is_seeded === 1,
                meta,
                createInstance: () => createLaunchInstance({ agent, blockId, overrides, instanceName, finalWorkDir }),
                setMeta: (m) => RpcApi.SetMetaCommand(TabRpcClient, { oref, meta: m }),
                // A terminal-tier CLI is started by its term view once the terminal is attached: one
                // that queries the terminal at startup (Aider) would otherwise wait for an answer forever.
                resync: () =>
                    provider.controllerType === "terminal"
                        ? Promise.resolve()
                        : RpcApi.ControllerResyncCommand(TabRpcClient, {
                              tabid: targetTabId ?? atoms.staticTabId(),
                              blockid: blockId,
                              forcerestart: true,
                          }),
                warn: (msg) => Logger.warn("agent", msg),
            });
            if (committed.ok !== false) markAgentOpen(blockId, "committed");
            if (committed.ok === false) {
                // Same as the catch below: a launch that didn't happen must not leave a focus
                // request for a later, unrelated mount.
                cancelComposerFocusRequest(blockId);
                this.launchError = committed.error;
                return false;
            }
            await linkLaunchAccount(agent, overrides, effectiveProvider);
            // Picking a conversation back up (My agents): once the pane runs, the agent says where it is. A fork
            // is a new branch, not a resume.
            if (launchResumesConversation(forkSessionArgs.continueSessionId, overrides?.forkSession)) {
                await requestCatchUp(blockId, "resumed").catch(() => {});
            }
            return true;
        } catch (e: any) {
            cancelComposerFocusRequest(blockId);
            Logger.error("agent", "Failed to launch agent definition", { error: String(e) });
            return false;
        }
    }

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
     *
     * Quick-fork lives right alongside it — both are per-agent actions on
     * THIS pane's own conversation, landing their result as a sibling tab
     * in this SAME pane (see `quick-fork.ts`'s doc comment for why this
     * replaced an earlier, wrong attempt that opened a new top-level
     * window tab instead). The items themselves are in agent-body-menu.ts.
     */
    getBodyContextMenuItems(): ContextMenuItem[] {
        const meta = this.blockAtom()?.meta;
        const agentId = meta?.["agentId"] as string | undefined;
        return agentBodyMenuItems({
            blockId: this.blockId,
            meta,
            hasDefinition: agentId != null && this.agentDefinitions().some((a) => a.id === agentId),
            openModal: this._openModal,
            quickFork: () => void quickForkAgent(this),
            openView: () => agentViewControls(this.blockId)?.setPopoverOpen(true),
            shutDown: () => void requestShutdown(this.blockId, (meta?.["agentName"] as string | undefined) || agentId || "this agent"),
        });
    }

    // Returns false (caller falls back to the block's dummy input) while no
    // composer is mounted — the picker screen, a history tab.
    giveFocus(): boolean {
        const ta = this.focusTargetRef.current;
        if (ta == null) return false;
        return focusComposer(ta);
    }

    dispose(): void {
        this.unregisterModel();
    }
}

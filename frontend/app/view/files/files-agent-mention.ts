// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Agents and the Files pane: which agent pane the user was last in, Alt+K's `@path` mentions, and "Attach to <agent>". */

import { paneWorkdir } from "@/app/drag/file-drop-actions";
import { dropPathsOnto, visibleDropTargets } from "@/app/drag/file-drop";
import { shortcutFor } from "@/app/keybindings";
import { getObjectValue, makeORef } from "@/app/store/mos";
import type { FsEntry } from "@/types/rpc/FsEntry";
import { isContainerPane, spliceComposerTokens } from "../agent/hooks/useAgentDropAttach";
import { errorText, type FilesModel } from "./files-model";
import { isWithin, samePath } from "./files-path";

/** The agent pane the user last worked in, for Alt+K: Files has the focus
 *  when the key is pressed, so it is remembered as focus moves. */
let lastAgentBlock: string | null = null;
let focusTracking = false;
export function trackAgentFocus(): void {
    if (focusTracking || typeof window === "undefined") return;
    focusTracking = true;
    window.addEventListener(
        "focusin",
        (e) => {
            const pane = e.target instanceof Element ? e.target.closest('[data-role="pane"][data-blockid]') : null;
            const id = pane?.getAttribute("data-blockid");
            if (id && getObjectValue<Block>(makeORef("block", id))?.meta?.view === "agent") lastAgentBlock = id;
        },
        true
    );
}

/** `@path` for an agent's composer: relative to its working folder when
 *  inside it, quoted when it has a space. */
export function mentionToken(path: string, workdir: string | undefined): string {
    let p = path;
    if (workdir && isWithin(path, workdir) && !samePath(path, workdir)) {
        p = path.slice(workdir.replace(/[\\/]+$/, "").length + 1);
    }
    return /\s/.test(p) ? `@"${p}"` : `@${p}`;
}


/** The agent panes on screen, by name: "Attach to <agent>" targets. */
export const agentTargets = (): { blockId: string; name: string }[] =>
    visibleDropTargets()
        .map((blockId) => ({ blockId, meta: getObjectValue<Block>(makeORef("block", blockId))?.meta }))
        .filter((t) => t.meta?.view === "agent")
        .map((t) => ({ blockId: t.blockId, name: (t.meta?.["agentName"] as string | undefined)?.trim() || "agent" }));

/**
 * Alt+K (spec §8.2, route 3): an `@path` mention of each selected entry
 * in the composer of the agent the user was last working with, as
 * Claude Code's own editor integrations do. Relative to the agent's
 * working folder when inside it.
 */
export const mentionInAgent = (model: FilesModel, list: FsEntry[]): void => {
    if (model.refuseInRoot("Mentioning files")) return;
    if (list.length === 0) return;
    // A host's paths mean nothing to an agent on this computer.
    if (model.connection()) {
        model.notOnHost("Mentioning files in an agent");
        return;
    }
    const agents = agentTargets();
    const target = agents.find((t) => t.blockId === lastAgentBlock) ?? (agents.length === 1 ? agents[0] : undefined);
    if (!target) {
        model.setStatus(
            {
                text: agents.length === 0 ? "No agent pane is open to mention these in." : `Click into the agent you mean first, then ${shortcutFor("files:mention")} here.`,
                tone: "info",
            },
            4000
        );
        return;
    }
    const workdir = paneWorkdir(target.blockId);
    // A container agent sees only its working folder (bind-mounted), so a
    // host path outside it means nothing to it (ReAgent on #4225).
    if (isContainerPane(target.blockId)) {
        const outside = list.filter((e) => !workdir || !isWithin(model.pathOf(e.name), workdir));
        if (outside.length > 0) {
            model.setStatus(
                {
                    text: `${target.name} runs in a container and sees only its working folder. Drop ${outside.length === 1 ? outside[0].name : "those files"} on it to copy ${outside.length === 1 ? "it" : "them"} in.`,
                    tone: "info",
                },
                6000
            );
            return;
        }
    }
    const tokens = list.map((e) => mentionToken(model.pathOf(e.name), workdir));
    const root = document.querySelector<HTMLElement>(`[data-role="pane"][data-blockid="${CSS.escape(target.blockId)}"]`);
    if (!root || !spliceComposerTokens(root, tokens)) {
        model.setStatus({ text: `${target.name}'s message box isn't open.`, tone: "error" }, 4000);
    }
};

export const attachToAgent = (model: FilesModel, target: { blockId: string; name: string }, list: FsEntry[]): void => {
    if (model.connection()) {
        model.notOnHost("Attaching files to an agent");
        return;
    }
    const paths = list.map((e) => model.pathOf(e.name));
    void dropPathsOnto(target.blockId, paths).then(
        (ok) =>
            model.setStatus(
                ok
                    ? { text: `Attached ${list.length === 1 ? list[0].name : `${list.length} items`} to ${target.name}`, tone: "info" }
                    : { text: `${target.name} can't take files right now`, tone: "error" },
                4000
            ),
        (err) => model.setStatus({ text: `Couldn't attach: ${errorText(err)}`, tone: "error" })
    );
};

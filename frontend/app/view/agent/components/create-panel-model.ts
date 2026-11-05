// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The create panel's props, its step names and the helpers its parts share. */

import { getApi } from "@/app/store/global";
import type { AgentDefinition } from "@/app/store/rpc-api";
import type { CreateDraft, CreatedAgent } from "./AgentCreateForm";

/** A system tool the harness needs that wasn't found, with where to get it. */
export interface MissingPrereq {
    tool: string;
    label: string;
    installUrl: string;
    installLinkText: string;
}

export type Step = "tools" | "install" | "create";

export const STEP_LABEL: Record<Step, string> = { tools: "Tools", install: "Install", create: "Create" };

export interface AgentCreatePanelProps {
    template: AgentDefinition;
    draft: CreateDraft;
    /** The picker's answer for this card's CLI: false = not found. */
    installed: boolean | undefined;
    /** System tools the harness needs that aren't on PATH; `fresh` re-asks. */
    probePrereqs: (fresh: boolean) => Promise<MissingPrereq[]>;
    /** Asks the picker to check the CLI again (after an install). */
    recheckInstalled: () => Promise<void>;
    /** Launches the agent the form created. */
    onCreated: (agent: CreatedAgent) => Promise<void>;
    onClose: () => void;
    /** Creating: the panel refuses to close meanwhile. */
    onBusyChange: (busy: boolean) => void;
}

export function open(url: string): void {
    try {
        getApi().openExternal(url);
    } catch {
        /* the link stays visible as text */
    }
}

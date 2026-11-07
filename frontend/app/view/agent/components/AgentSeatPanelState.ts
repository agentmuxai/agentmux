// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Seat panel's state: the agent as listed, the form (kept by the list across a rebuild), the options the role
 * and lead pickers offer, and the save and clear calls.
 */

import { createEffect, createMemo, createSignal } from "solid-js";

import type { SelectOption } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentSeat } from "@/types/rpc/AgentSeat";
import { SEAT_ROLES, type SeatRole } from "../../../agent-model/seat-format";
import { sortByLastUsed } from "../agent-recency";
import { useAgentList } from "../use-agent-list";

/** Shown when srv refuses a channel-local agent (§2): it has no shared definition to hold a seat. */
export const NO_SHARED_DEFINITION_NOTE =
    "This agent exists only in this channel, with no shared definition, so it can't have a seat.";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/**
 * What the seat form holds. A panel that gets its own from the list keeps its edits when the list rebuilds it
 * (a resize that moves the panel to another row remounts it); one that is given none makes its own.
 */
export const createSeatForm = () => {
    const [role, setRole] = createSignal<SeatRole | "">("");
    const [qualifier, setQualifier] = createSignal("");
    const [lead, setLead] = createSignal("");
    const [area, setArea] = createSignal("");
    const [busy, setBusy] = createSignal<"save" | "clear" | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [saved, setSaved] = createSignal<string | null>(null);
    // The seat the fields were last filled from, so a rebuilt panel does not fill over the edits.
    let filledFrom: string | undefined;
    return {
        role,
        setRole,
        qualifier,
        setQualifier,
        lead,
        setLead,
        area,
        setArea,
        busy,
        setBusy,
        error,
        setError,
        saved,
        setSaved,
        filledFrom: () => filledFrom,
        setFilledFrom: (key: string) => {
            filledFrom = key;
        },
    };
};
export type SeatForm = ReturnType<typeof createSeatForm>;

export interface AgentSeatPanelProps {
    agentId: string;
    agentName: string;
    /** Given when hosted in a modal (My Agents' "Seat…"): shows a Close button. */
    onClose?: () => void;
    /** The list's copy of the form, so a rebuild of the panel keeps the edits. */
    form?: SeatForm;
}

export type AgentSeatState = ReturnType<typeof createAgentSeat>;

export function createAgentSeat(props: AgentSeatPanelProps) {
    const [agents, loadError] = useAgentList();
    const self = createMemo(() => agents()?.find((a) => a.id === props.agentId));
    /** Agents only, not templates: what a lead or a roster line can name. */
    const people = createMemo(() => (agents() ?? []).filter((a) => a.is_seeded === 0));
    const seat = createMemo<AgentSeat | undefined>(() => self()?.seat);

    const form = props.form ?? createSeatForm();
    const { role, setRole, qualifier, setQualifier, lead, setLead, area, setArea } = form;
    const { setBusy, setError, setSaved, filledFrom, setFilledFrom } = form;
    const [importing, setImporting] = createSignal(false);

    // Fill the form from the seat as listed, and again whenever that seat
    // changes (a save, a clear, a roster import, another window).
    createEffect(() => {
        if (agents() === undefined) return;
        const s = seat();
        const key = JSON.stringify(s ?? null);
        if (key === filledFrom()) return;
        setFilledFrom(key);
        setRole(s?.role ?? "");
        setQualifier(s?.qualifier ?? "");
        setLead(s?.lead ?? "");
        setArea(s?.area ?? "");
    });

    const roleOptions = (): SelectOption[] => [
        { value: "", label: "Choose a role…", disabled: true },
        ...SEAT_ROLES.map((r) => ({ value: r.value, label: r.label })),
    ];
    const leadOptions = createMemo((): SelectOption[] => {
        const options = sortByLastUsed(
            people().filter((a) => a.id !== props.agentId),
            (a) => a.name
        ).map((a) => ({ value: a.id, label: a.name }));
        const current = lead();
        if (current && !options.some((o) => o.value === current)) {
            options.unshift({ value: current, label: "An agent not on this machine" });
        }
        return [{ value: "", label: "The operator" }, ...options];
    });

    const run = async (kind: "save" | "clear"): Promise<void> => {
        setError(null);
        setSaved(null);
        setBusy(kind);
        try {
            const r = role();
            const input =
                kind === "clear" || r === ""
                    ? null
                    : { role: r, qualifier: qualifier().trim(), lead: lead(), area: area().trim() };
            await RpcApi.AgentSetSeatCommand(TabRpcClient, { agent_id: props.agentId, seat: input });
            setSaved(kind === "save" ? "Seat saved." : "Seat cleared.");
        } catch (e) {
            const text = errorText(e);
            setError(/no shared definition/.test(text) ? NO_SHARED_DEFINITION_NOTE : text);
        } finally {
            setBusy(null);
        }
    };

    return {
        agents,
        loadError,
        self,
        people,
        seat,
        form,
        importing,
        setImporting,
        roleOptions,
        leadOptions,
        run,
    };
}

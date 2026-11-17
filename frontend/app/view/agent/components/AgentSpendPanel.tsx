// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSpendPanel: the "Spend cap…" view of My Agents' row panel. What the agent has used today and its daily cap, and
 * the person's choices at any time: a number of dollars, no cap, or the default every agent without a cap of its own has
 * (a setting, Settings > Advanced). One `agent.spend` call each, by the agent's id, so it works for an agent with no
 * pane open. An agent can only lower its own cap (`Agents update spend_cap_usd`); this is the person's.
 */

import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

import { Button, Field, TextInput } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentSpendResult } from "@/types/rpc/AgentSpendResult";
import { cents, parseCapInput } from "../spend-cap-input";
import "./AgentPrimitiveModal.scss";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));
const boxText = (r: AgentSpendResult): string => (r.cap_usd === null ? "" : String(cents(r.cap_usd)));

/** What the panel says about the agent's day and cap, in one line. */
export function capLine(r: AgentSpendResult): string {
    const spent = cents(r.spent_usd);
    if (r.cap_usd === null) return t(r.own_cap ? "agentSpend.line.ownNone" : "agentSpend.line.defaultNone", { spent });
    return t(r.own_cap ? "agentSpend.line.own" : "agentSpend.line.default", { spent, cap: cents(r.cap_usd) });
}

/** The panel's state: the agent's cap as srv last said it, the box a person types in, and the calls that change it. */
function createSpendCap(agentId: string) {
    const [state, setState] = createSignal<AgentSpendResult | null>(null);
    const [draft, setDraft] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const [saved, setSaved] = createSignal(false);
    /** The text the box held when the cap was last read: a box that differs holds an edit a reload must not take. */
    let shownText = "";

    const call = (action: string, capUsd: number | null): Promise<AgentSpendResult> =>
        RpcApi.AgentSpendCommand(TabRpcClient, { blockid: "", agent: agentId, action, cap_usd: capUsd });
    const load = async (): Promise<void> => {
        try {
            const r = await call("get", null);
            if (draft() === shownText) setDraft(boxText(r));
            shownText = boxText(r);
            setState(r);
            setError(null);
        } catch (e) {
            setError(errorText(e));
        }
    };
    const change = async (action: string, capUsd: number | null): Promise<void> => {
        setBusy(true);
        setError(null);
        setSaved(false);
        try {
            const r = await call(action, capUsd);
            shownText = boxText(r);
            setDraft(shownText);
            setState(r);
            setSaved(true);
        } catch (e) {
            setError(errorText(e));
        } finally {
            setBusy(false);
        }
    };
    const setFromBox = (): void => {
        const input = parseCapInput(draft());
        if (input.ok === false) {
            setError(t(`agentSpend.err.${input.reason}`));
            return;
        }
        void change("set", input.usd);
    };
    onMount(() => {
        void load();
        onCleanup(onMuxEvent(WpsEvent.AgentContentChanged, () => void load()));
    });
    return { state, draft, setDraft, busy, error, saved, setSaved, load, change, setFromBox };
}
type SpendCap = ReturnType<typeof createSpendCap>;

/** The line about the day and the box for the cap; for an agent on a login, only that no cap applies. */
function CapForm(props: { c: SpendCap; r: AgentSpendResult; agentName: string }): JSX.Element {
    const { c } = props;
    return (
        <Show
            when={props.r.applies}
            fallback={
                <p class="agent-primitive-modal-global-note" data-testid="agent-spend-none">
                    {t("agentSpend.login", { name: props.agentName })}
                </p>
            }
        >
            <p class="agent-primitive-modal-global-note" data-testid="agent-spend-line">
                {capLine(props.r)}
            </p>
            <Field label={t("agentSpend.label")} layout="stacked" hint={t("agentSpend.hint")}>
                <TextInput
                    inputmode="decimal"
                    placeholder={t("agentSpend.placeholder")}
                    value={c.draft()}
                    disabled={c.busy()}
                    data-testid="agent-spend-input"
                    onInput={(e) => {
                        c.setDraft(e.currentTarget.value);
                        c.setSaved(false);
                    }}
                    onKeyDown={(e) => {
                        if (e.key === "Enter") c.setFromBox();
                    }}
                />
            </Field>
        </Show>
    );
}

/** Set the box's dollars, no cap, or the default; each is one call. */
function CapActions(props: { c: SpendCap; r: AgentSpendResult }): JSX.Element {
    const { c } = props;
    const defaultCap = () => props.r.default_cap_usd;
    const alreadyNone = () => props.r.own_cap && props.r.cap_usd === null;
    return (
        <>
            <Button disabled={c.busy() || !props.r.own_cap} onClick={() => void c.change("default", null)} data-testid="agent-spend-default">
                {defaultCap() === null ? t("agentSpend.useDefaultNone") : t("agentSpend.useDefault", { cap: cents(defaultCap()!) })}
            </Button>
            <Button disabled={c.busy() || alreadyNone()} onClick={() => void c.change("set", null)} data-testid="agent-spend-nocap">
                {t("agentSpend.noCap")}
            </Button>
            <Button tone="accent" busy={c.busy()} disabled={c.busy()} onClick={c.setFromBox} data-testid="agent-spend-set">
                {t("agentSpend.set")}
            </Button>
        </>
    );
}

interface AgentSpendPanelProps {
    agentId: string;
    agentName: string;
    /** Given when hosted in the row panel: shows a Close button. */
    onClose?: () => void;
}

export const AgentSpendPanel = (props: AgentSpendPanelProps): JSX.Element => {
    const c = createSpendCap(props.agentId);
    return (
        <div class="agent-primitive-modal-detail agent-spend-panel" data-testid="agent-spend-panel">
            <div class="agent-primitive-modal-readonly">
                <p class="agent-primitive-modal-global-note">{t("agentSpend.intro", { name: props.agentName })}</p>
                <Show when={!c.state() && !c.error()}>
                    <p class="agent-primitive-modal-global-note" role="status">
                        {t("agentSpend.loading")}
                    </p>
                </Show>
                <Show when={c.state()}>{(r) => <CapForm c={c} r={r()} agentName={props.agentName} />}</Show>
                <Show when={c.error()}>
                    <div class="agent-primitive-modal-error" role="alert">
                        {c.error()}
                    </div>
                </Show>
                <Show when={!c.state() && c.error()}>
                    <div class="agent-primitive-modal-actions">
                        <Button onClick={() => void c.load()}>{t("agentSpend.retry")}</Button>
                    </div>
                </Show>
                <Show when={c.saved()}>
                    <p class="agent-primitive-modal-global-note" role="status" data-testid="agent-spend-saved">
                        {t("agentSpend.saved")}
                    </p>
                </Show>
                <div class="agent-primitive-modal-actions">
                    <Show when={props.onClose}>{(close) => <Button onClick={() => close()()}>{t("agentSpend.close")}</Button>}</Show>
                    <Show when={c.state()?.applies ? c.state() : null}>{(r) => <CapActions c={c} r={r()} />}</Show>
                </div>
            </div>
        </div>
    );
};

AgentSpendPanel.displayName = "AgentSpendPanel";

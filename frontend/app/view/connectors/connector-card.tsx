// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One connector's card in Connectors' list (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md §4.2): an icon, the
// name, one line, and one control. A card that is not connected says what the connector is for and has Connect; a
// connected one says its state in words and opens its details when clicked. The disclosure and the control are
// separate buttons, never nested (§4.7). The card's layout follows its own width (connector-card.scss), not the
// window's or the column count.

import { Show, type JSX } from "solid-js";

import { t } from "@/app/i18n/i18n";
import { Button, StatusDot } from "@/app/element/ui";
import { ProviderLogo } from "@/element/ProviderLogo";
import type { ConnectorEntry } from "./connector-entries";
import "./connector-card.scss";

export interface ConnectorCardProps {
    entry: ConnectorEntry;
    /** Its details (or, before it is connected, its Connect section) are showing. */
    open: boolean;
    /** It has just connected: it shows "Connected" in place for a moment (§4.9). */
    flash?: boolean;
    /** The id of the panel it opens, for `aria-controls`. */
    detailId: string;
    onToggle: () => void;
    onReconnect: () => void;
}

function Control(props: ConnectorCardProps): JSX.Element {
    const e = () => props.entry;
    const label = () => {
        switch (e().control) {
            case "connect":
                return t("connectors.control.connect");
            case "howto":
                return t("connectors.control.howTo");
            default:
                return t("connectors.control.reconnect");
        }
    };
    const named = () => {
        switch (e().control) {
            case "connect":
                return t("connectors.control.connectNamed", { name: e().name });
            case "howto":
                return t("connectors.control.howToNamed", { name: e().name });
            default:
                return t("connectors.control.reconnectNamed", { name: e().name });
        }
    };
    return (
        <Show
            when={e().control !== "open"}
            fallback={
                <Show
                    when={props.flash}
                    fallback={<span class="connector-card-chevron" aria-hidden="true" data-open={props.open ? "" : undefined} />}
                >
                    <span class="connector-card-flash" aria-hidden="true">
                        {t("connectors.flash.connected")}
                    </span>
                </Show>
            }
        >
            <Button
                density="compact"
                tone={e().control === "howto" ? "neutral" : "accent"}
                icon={e().control === "reconnect" ? "rotate-right" : undefined}
                aria-label={named()}
                aria-expanded={e().control === "reconnect" ? undefined : props.open ? "true" : "false"}
                aria-controls={e().control === "reconnect" ? undefined : props.detailId}
                onClick={() => (e().control === "reconnect" ? props.onReconnect() : props.onToggle())}
            >
                {label()}
            </Button>
        </Show>
    );
}

export function ConnectorCard(props: ConnectorCardProps): JSX.Element {
    const e = () => props.entry;
    const text = (): JSX.Element => (
        <>
            <span class="connector-card-name">{e().name}</span>
            <span class="connector-card-line" data-tone={e().connected ? e().tone : undefined}>
                <Show when={e().connected} fallback={e().line}>
                    <StatusDot tone={e().tone} label={e().state ?? ""} />
                    {e().state}
                </Show>
            </span>
        </>
    );
    return (
        <div
            class="connector-card"
            classList={{ "is-open": props.open, "is-flash": props.flash, "is-attention": e().connected && e().tone === "error", "is-connected": e().connected }}
            data-connector={e().key}
            // The whole connected card is the disclosure for a mouse; the keyboard and screen readers use its button.
            onClick={(ev) => {
                if (e().connected && !(ev.target as Element).closest("button")) props.onToggle();
            }}
        >
            <span class="connector-card-icon" aria-hidden="true">
                <ProviderLogo provider={e().tile.id} size={20} />
            </span>
            <Show when={e().connected} fallback={<div class="connector-card-text">{text()}</div>}>
                <Button
                    variant="row"
                    class="connector-card-text connector-card-disclosure"
                    aria-expanded={props.open ? "true" : "false"}
                    aria-controls={props.detailId}
                    onClick={(ev) => {
                        ev.stopPropagation();
                        props.onToggle();
                    }}
                >
                    {text()}
                </Button>
            </Show>
            <span class="connector-card-control">
                <Control {...props} />
            </span>
            {/* Announced once, when the card connects; the visible flash is aria-hidden so it is not read twice. */}
            <Show when={props.flash}>
                <span class="connector-card-announce" role="status">
                    {t("connectors.flash.announce", { name: e().name })}
                </span>
            </Show>
        </div>
    );
}

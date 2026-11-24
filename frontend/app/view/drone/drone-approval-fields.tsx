// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Approval block's fields on the canvas: what is proposed (the graph's own words) and the items the card shows,
// one per line as `Label | {{reference}}`.

import { createSignal, type JSX } from "solid-js";

import { TextArea, TextInput } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import type { FlowNode } from "./drone-types";

type Item = { label: string; value: string };

/** The `show` list as the lines the person edits. */
export function showToText(show: unknown): string {
    if (!Array.isArray(show)) return "";
    return (show as Item[]).map((i) => `${i.label ?? ""} | ${i.value ?? ""}`).join("\n");
}

/** The lines the person typed, as the `show` list: text before the first `|` is the label. */
export function textToShow(text: string): Item[] {
    return text
        .split("\n")
        .filter((line) => line.trim() !== "")
        .map((line) => {
            const bar = line.indexOf("|");
            return bar < 0 ? { label: "", value: line.trim() } : { label: line.slice(0, bar).trim(), value: line.slice(bar + 1).trim() };
        });
}

const Row = (p: { label: string; children: JSX.Element }): JSX.Element => (
    <label class="drone-node-field">
        <span class="drone-node-field-label">{p.label}</span>
        {p.children}
    </label>
);

export const ApprovalFields = (p: { node: FlowNode; update: (patch: Record<string, unknown>) => void }): JSX.Element => {
    // The lines as typed: a new empty line has to stay until it has text.
    const [text, setText] = createSignal(showToText(p.node.data["show"]));
    return (
    <>
        <Row label={t("drone.approval.proposed")}>
            <TextInput
                variant="plain"
                class="drone-input nodrag"
                value={(p.node.data["title"] as string) ?? ""}
                onInput={(e) => p.update({ title: e.currentTarget.value })}
                onContextMenu={showTextInputContextMenu}
                placeholder={t("drone.approval.proposedPlaceholder")}
            />
        </Row>
        <Row label={t("drone.approval.shows")}>
            <TextArea
                variant="plain"
                class="drone-input drone-input--grow nodrag nowheel"
                value={text()}
                onInput={(e) => {
                    setText(e.currentTarget.value);
                    p.update({ show: textToShow(e.currentTarget.value) });
                }}
                onContextMenu={showTextInputContextMenu}
                placeholder={t("drone.approval.showsPlaceholder")}
            />
        </Row>
    </>
    );
};

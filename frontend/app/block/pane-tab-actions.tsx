// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane type's own buttons (the agent's Find and Messages, the terminal's Start and Restart, the Vault's Import
 * and Export), drawn inside the selected tab, right of its name. The view model declares them as it always did
 * (`endIconButtons`); the tab strip draws this only for the selected tab, so they appear on the tab when it is
 * selected and leave when another is. The header's right side keeps the pane's own controls (grip, minimize,
 * maximize, close) and the + for a new tab.
 */

import { createMemo, For, Show, type JSX } from "solid-js";

import { DeclIconButton } from "@/app/element/ui/DeclIconButton";
import { t } from "@/app/i18n/i18n";
import { NodeModel } from "@/layout/index";
import * as util from "@/util/util";

import "./pane-tab-actions.scss";

/** A press on a button is a press on the button, not on the tab: it does not select, drag, rename or open the tab's menu. */
const keepFromTab = (e: Event) => e.stopPropagation();

export function PaneTabActions(props: { viewModel: ViewModel | null; nodeModel: NodeModel }): JSX.Element {
    const buttons = createMemo(() => util.useAtomValueSafe(props.viewModel?.endIconButtons));
    const closing = () => props.nodeModel.isClosing?.() ?? false;
    return (
        <Show when={!closing() && (buttons()?.length ?? 0) > 0}>
            <div
                class="pane-tab-actions"
                role="group"
                aria-label={t("pane.tab.actions")}
                onMouseDown={keepFromTab}
                onClick={keepFromTab}
                onDblClick={keepFromTab}
                onContextMenu={keepFromTab}
                draggable={false}
            >
                <For each={buttons()}>{(button) => <DeclIconButton decl={button} className="pane-tab-action" />}</For>
            </div>
        </Show>
    );
}

/** The class a button carries when it is the only way in to something (the agent's Messages): it is never trimmed away. */
const KEEP_CLASS = "block-frame-end-keep";

/**
 * The buttons that must stay reachable when the header is too narrow to show its tabs (a narrow tier hides the tab
 * strip, and the selected tab's buttons with it): only those flagged `block-frame-end-keep`, in the header's range.
 * Hidden at every other width, where the same buttons are in the selected tab (mixins.scss).
 */
export function KeptEndIcons(props: { viewModel: ViewModel | null; nodeModel: NodeModel }): JSX.Element {
    const kept = createMemo(() =>
        (util.useAtomValueSafe(props.viewModel?.endIconButtons) ?? []).filter((b) => (b.className ?? "").includes(KEEP_CLASS))
    );
    const closing = () => props.nodeModel.isClosing?.() ?? false;
    return (
        <Show when={!closing() && kept().length > 0}>
            <div class="block-frame-end-kept" onDblClick={keepFromTab}>
                <For each={kept()}>{(button) => <DeclIconButton decl={button} />}</For>
            </div>
        </Show>
    );
}

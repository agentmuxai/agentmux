// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What a browser pane says about popup windows: the list of its open popups, and where this pane's own popup came from. */

import { For, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import type { BrowserViewModel } from "./browser-model";

export function BrowserPopupNotes(props: { model: BrowserViewModel }): JSX.Element {
    const model = props.model;
    return (
        <>
            <Show when={model.popupWindowsAtom().length > 0}>
                <div class="browser-popup-windows" role="list" aria-label="Popup windows">
                    <For each={model.popupWindowsAtom()}>
                        {(w) => (
                            <div class="browser-popup-window" role="listitem">
                                <i class="fa-solid fa-window-restore" aria-hidden="true" />
                                <span class="browser-popup-window-url" {...tip(w.url)}>
                                    Popup window: {w.url}
                                </span>
                                <Button
                                    density="compact"
                                    title="Bring this popup window to the front"
                                    onClick={() => model.showPopup(w.id).catch(() => {})}
                                >
                                    Show
                                </Button>
                                <Button
                                    density="compact"
                                    title="Close this popup window"
                                    onClick={() => model.closePopup(w.id).catch(() => {})}
                                >
                                    Close
                                </Button>
                            </div>
                        )}
                    </For>
                </div>
            </Show>
            <Show when={model.popupFromAtom()}>
                {(from) => (
                    <div class="browser-popup-from" role="note">
                        <i class="fa-solid fa-window-restore" aria-hidden="true" />
                        <span>
                            Popup from <b>{from()}</b>
                        </span>
                    </div>
                )}
            </Show>
        </>
    );
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Downloads button in a browser pane's address bar: a menu of the downloads the pane has finished, newest
// first, so a file saved a while ago can still be found after the strip's row has folded away
// (docs/specs/SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md §5). Choosing a saved one shows it in its folder, the
// same call as the strip's Show in folder. It only appears once the pane has had a download, and its list
// comes only from the host's events, never from the page.

import { Button, type MenuEntry, MenuTrigger } from "@/app/element/ui";
import { Show, type JSX } from "solid-js";
import type { BrowserViewModel } from "./browser-model";
import { pastStatus, type DownloadRow } from "./downloads";

/** One finished download as a menu line: its name, what became of it, and, when it was saved, a click that shows it. */
export function pastEntry(row: DownloadRow, reveal: (path: string) => void): MenuEntry {
    const path = row.state === "done" ? row.path : undefined;
    return {
        label: row.name,
        sublabel: pastStatus(row),
        icon: path ? "folder-open" : undefined,
        disabled: !path,
        onSelect: path ? () => reveal(path) : undefined,
        testId: "browser-past-download",
    };
}

export function DownloadsButton(props: { model: BrowserViewModel; pane: () => HTMLElement | null | undefined }): JSX.Element {
    const model = props.model;
    const items = (): MenuEntry[] => [
        ...model.pastDownloadsAtom().map((row) => pastEntry(row, (path) => model.revealDownload(path))),
        { label: "", divider: true },
        { label: "Clear this list", onSelect: () => model.clearPastDownloads(), testId: "browser-past-downloads-clear" },
    ];
    const title = "Downloads: choose one to show it in its folder";
    return (
        <Show when={model.pastDownloadsAtom().length > 0}>
            <MenuTrigger items={items()} placement="bottom-end" containWithin={props.pane} alignToAnchorSide>
                <Button
                    tone="quiet"
                    class="ui-icon-button browser-nav-btn browser-downloads-btn"
                    data-testid="browser-downloads-button"
                    title={title}
                    aria-label={title}
                >
                    <i class="fa-solid fa-download" aria-hidden="true" />
                </Button>
            </MenuTrigger>
        </Show>
    );
}

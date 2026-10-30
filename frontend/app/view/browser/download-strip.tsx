// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The strip under the address bar that shows a pane's downloads: name,
// progress, Cancel while one runs, Show in folder when it is done
// (docs/specs/SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md §5). It is pane
// chrome, outside the page's area, and driven only by the host's events, so a
// page can't imitate it. Text rows, no icons.

import { Button } from "@/app/element/ui";
import { For, Show, type JSX } from "solid-js";
import type { BrowserViewModel } from "./browser-model";
import { progressText, type DownloadRow } from "./downloads";

function RowStatus(props: { row: DownloadRow }): JSX.Element {
    const row = () => props.row;
    return (
        <>
            <Show when={row().state === "running"}>
                <span class="browser-download-progress">{progressText(row())}</span>
            </Show>
            <Show when={row().state === "done"}>
                <span class="browser-download-progress">{row().reason ?? "saved"}</span>
            </Show>
            <Show when={row().state === "cancelled"}>
                <span class="browser-download-progress">cancelled</span>
            </Show>
            <Show when={row().state === "failed"}>
                <span class="browser-download-failed">{row().reason ?? "The download failed."}</span>
            </Show>
        </>
    );
}

export function DownloadStrip(props: { model: BrowserViewModel }): JSX.Element {
    const model = props.model;
    return (
        <Show when={model.downloadsAtom().length > 0}>
            <div class="browser-downloads" role="status" aria-label="Downloads">
                <For each={model.downloadsAtom()}>
                    {(row) => (
                        <div class="browser-download-row" classList={{ "browser-download-row--failed": row.state === "failed" }}>
                            <Show
                                when={row.state !== "ignored"}
                                fallback={<span class="browser-download-failed">{row.reason}</span>}
                            >
                                <span class="browser-download-name">{row.name}</span>
                                <RowStatus row={row} />
                                <span class="browser-download-actions">
                                    <Show when={row.state === "running" && row.id != null}>
                                        <Button
                                            density="compact"
                                            class="browser-download-cancel"
                                            title="Stop this download"
                                            onClick={() => model.cancelDownload(row.id as number)}
                                        >
                                            Cancel
                                        </Button>
                                    </Show>
                                    <Show when={row.state === "done" && row.path}>
                                        <Button
                                            density="compact"
                                            class="browser-download-reveal"
                                            title="Show the file in its folder"
                                            onClick={() => model.revealDownload(row.path as string)}
                                        >
                                            Show in folder
                                        </Button>
                                    </Show>
                                </span>
                            </Show>
                            <Show when={row.state !== "running"}>
                                <Button
                                    density="compact"
                                    class="browser-download-dismiss"
                                    title="Hide this line"
                                    aria-label="Hide this download line"
                                    onClick={() => model.dismissDownload(row.key)}
                                >
                                    Hide
                                </Button>
                            </Show>
                        </div>
                    )}
                </For>
            </div>
        </Show>
    );
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's body for a tab whose viewer isn't text: the image, audio and
 * video, or info card viewer, with the shared toolbar. Only the tab in
 * front is mounted (a video behind another tab stops), and a tab's viewer
 * remounts when the tab or its viewer changes, not on a follow-folder swap.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.1, §4.3.
 */

import { pushNotification } from "@/app/store/global";
import { createMemo, Match, Show, Switch, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import type { EditorViewModel } from "../editor-model";
import { createFollowFolder } from "./follow-folder";
import { AvViewer } from "./media/av-viewer";
import { ImageViewer } from "./image/image-viewer";
import { InfoCard } from "./info/info-card";
import { ViewerBar, type ViewerBarProps } from "./viewer-bar";
import { TEXT_MAX_BYTES, type ViewerId } from "./viewer-choice";
import { droppedFileOf, isDroppedPath, neighbourInFolder } from "./viewer-tabs";
import "./viewers.scss";

export function ViewerHost(props: { model: EditorViewModel }): JSX.Element {
    const model = props.model;
    // Remount on a new tab or a new viewer; a path change inside the tab
    // (follow-folder, [ / ]) only moves the viewer to the new file.
    const key = createMemo(() => {
        const tab = model.activeTabAtom();
        return tab ? `${tab.id}|${model.activeViewer()}` : "";
    });
    return (
        <Show when={key()} keyed>
            {(k: string) => {
                const [tabId, viewer] = k.split("|");
                return <ViewerFor model={model} tabId={tabId} viewer={viewer as ViewerId} />;
            }}
        </Show>
    );
}

/** The switch over the text editor for a file that isn't text (an image or
 *  binary shown as text): the only way back to its own viewer. A text file
 *  shown as text gets none. */
export function TextViewerSwitch(props: { model: EditorViewModel }): JSX.Element {
    const model = props.model;
    const tabId = () => model.activeTabAtom()?.id ?? null;
    const options = (): ViewerId[] => {
        const probe = model.viewerTabs.probe(tabId());
        return probe && probe.kind !== "text" ? model.viewerTabs.options(tabId(), "text") : [];
    };
    const reopen = (v: ViewerId) => {
        const id = tabId();
        if (id) model.reopenWith(id, v);
    };
    return (
        <Show when={options().length > 1}>
            <ViewerBar viewer="text" options={options()} onReopenWith={reopen} />
        </Show>
    );
}

function ViewerFor(props: { model: EditorViewModel; tabId: string; viewer: ViewerId }): JSX.Element {
    const model = props.model;
    const tab = () => model.tabsAtom().find((t) => t.id === props.tabId);
    const path = () => tab()?.filePath ?? "";
    const dropped = () => isDroppedPath(path());
    const file = () => (dropped() ? droppedFileOf(path()) : undefined);
    const probe = () => model.viewerTabs.probe(props.tabId);
    const revision = () => model.viewerTabs.revision(props.tabId);
    const openWithSystem = () =>
        void RpcApi.FsOpenCommand(TabRpcClient, { path: path() }).catch((e) =>
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: "Couldn't open the file",
                message: e instanceof Error ? e.message : String(e),
                timestamp: new Date().toISOString(),
                type: "error",
                expiration: Date.now() + 8000,
            })
        );
    const follows = () => !!tab()?.followFolder;
    const canFollow = () => !dropped() && (props.viewer === "image" || props.viewer === "media");

    createFollowFolder({
        blockId: model.blockId,
        path,
        enabled: () => canFollow() && follows(),
        onNewer: (p) => model.retargetTab(props.tabId, p),
        onRewritten: () => {},
    });

    const bar = (): Omit<ViewerBarProps, "start" | "facts"> => ({
        viewer: props.viewer,
        options: model.viewerTabs.options(props.tabId, props.viewer),
        onReopenWith: (v) => model.reopenWith(props.tabId, v),
        followFolder: canFollow() ? { on: follows(), toggle: () => model.setFollowFolder(props.tabId, !follows()) } : undefined,
        onReveal: dropped() ? undefined : () => void model.revealInExplorer(path()),
        onOpenWithSystem: dropped() ? undefined : openWithSystem,
    });
    const mime = () => probe()?.mime || file()?.type || "";

    return (
        <Switch>
            <Match when={props.viewer === "image"}>
                <ImageViewer
                    path={dropped() ? "" : path()}
                    file={file()}
                    revision={revision()}
                    mime={mime()}
                    size={probe()?.size}
                    bar={bar()}
                    onStep={dropped() ? undefined : (d) => void neighbourInFolder(path(), props.viewer, d).then((p) => p && model.retargetTab(props.tabId, p)).catch(() => {})}
                />
            </Match>
            <Match when={props.viewer === "media"}>
                <AvViewer
                    path={dropped() ? "" : path()}
                    file={file()}
                    revision={revision()}
                    mime={mime() || "video/mp4"}
                    size={probe()?.size}
                    bar={bar()}
                />
            </Match>
            <Match when={props.viewer === "info"}>
                <InfoCard
                    path={path()}
                    file={file()}
                    probe={probe()}
                    bar={bar()}
                    note={
                        probe()?.kind === "text" && (probe()?.size ?? 0) > TEXT_MAX_BYTES
                            ? "This text file is larger than the editor opens (10 MB)."
                            : probe()?.kind === "pdf"
                              ? "PDF pages show here once the PDF viewer lands; until then, open it with the system's app."
                              : undefined
                    }
                    onOpenAsText={dropped() ? undefined : () => model.reopenWith(props.tabId, "text")}
                    onOpenWithSystem={dropped() ? undefined : openWithSystem}
                    onReveal={dropped() ? undefined : () => void model.revealInExplorer(path())}
                    onCopyPath={dropped() ? undefined : () => void clipboardWriteText(path()).catch(() => {})}
                />
            </Match>
        </Switch>
    );
}

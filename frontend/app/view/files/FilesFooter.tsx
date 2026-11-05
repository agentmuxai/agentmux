// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Below the Files pane's list: the progress of running copies and moves, the conflict question, and the status line. */

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { formatBytes } from "@/util/format-bytes";
import { createSignal, For, Show, type JSX } from "solid-js";
import type { FilesModel } from "./files-model";
import { opProgressText } from "./files-ops";
import { gitSummary } from "./files-view-text";

export function FilesFooter(props: { model: FilesModel; summary: () => string }): JSX.Element {
    const model = props.model;
    const summary = props.summary;
    return (
        <>
            <For each={model.ops.ops().filter((o) => o.state === "running" || o.state === "conflict")}>
                {(op) => (
                    <div class="files-op" role="status">
                        <span class="files-op-text">{opProgressText(op)}</span>
                        <span class="files-op-bar" aria-hidden="true">
                            <span
                                class="files-op-fill"
                                style={{ width: `${op.total_bytes > 0 ? Math.floor((op.done_bytes / op.total_bytes) * 100) : op.total_items > 0 ? Math.floor((op.done_items / op.total_items) * 100) : 0}%` }}
                            />
                        </span>
                        <Button tone="quiet" class="files-status-undo" onClick={() => void model.ops.cancel(op.op_id)}>
                            Cancel
                        </Button>
                    </div>
                )}
            </For>
            <Show when={model.ops.ops().find((o) => o.state === "conflict" && o.conflict)}>
                {(op) => <ConflictDialog op={op()} onResolve={(choice, all) => void model.ops.resolve(op().op_id, choice, all)} onCancel={() => void model.ops.cancel(op().op_id)} />}
            </Show>
            <div class="files-status" role="status">
                <Show
                    when={model.status()}
                    fallback={
                        <span>
                            {summary()}
                            <Show when={model.git()}>
                                {(g) => (
                                    <span class="files-git-summary" {...tip("git branch, commits ahead/behind its upstream, and changes under this folder")}>
                                        <i class="fa fa-code-branch" aria-hidden="true" /> {gitSummary(g())}
                                    </span>
                                )}
                            </Show>
                        </span>
                    }
                >
                    {(msg) => (
                        <span classList={{ "files-status-error": msg().tone === "error" }}>
                            {msg().text}
                            <Show when={msg().undo}>
                                <Button tone="quiet" class="files-status-undo" onClick={() => void model.undo()}>
                                    Undo
                                </Button>
                            </Show>
                        </span>
                    )}
                </Show>
            </div>
        </>
    );
}

/** A copy or move found something already at the destination (§7.2). */
function ConflictDialog(props: {
    op: import("./files-ops").OpView;
    onResolve: (choice: "replace" | "skip" | "keep_both", applyToAll: boolean) => void;
    onCancel: () => void;
}): JSX.Element {
    const [all, setAll] = createSignal(false);
    const c = () => props.op.conflict!;
    const name = () => c().dest.split(/[\\/]/).pop() ?? c().dest;
    const side = (size?: number, mtime?: number) =>
        [size != null ? formatBytes(size) : null, mtime != null ? new Date(mtime).toLocaleString() : null].filter(Boolean).join(", ");
    let first: HTMLButtonElement | undefined;
    return (
        <Modal open={true} scope="pane" size="sm" panelClass="files-conflict-panel" closeOnBackdropClick={false} onClose={props.onCancel} initialFocus={() => first ?? null}>
            <ModalHeader title={`“${name()}” already exists here`} />
            <ModalBody>
                <div class="files-conflict-body">
                    <div>
                        {c().dest_is_dir ? "A folder" : "A file"} with that name is already in the destination
                        {c().source_is_dir && c().dest_is_dir ? "." : `${c().dest_is_dir !== c().source_is_dir ? `, and the one being ${props.op.kind === "move" ? "moved" : "copied"} is a ${c().source_is_dir ? "folder" : "file"}.` : "."}`}
                    </div>
                    <Show when={!c().source_is_dir && !c().dest_is_dir}>
                        <div class="files-conflict-sides">
                            <div>Existing: {side(c().dest_size ?? undefined, c().dest_mtime ?? undefined)}</div>
                            <div>Incoming: {side(c().source_size ?? undefined, c().source_mtime ?? undefined)}</div>
                        </div>
                    </Show>
                    <label class="files-conflict-all">
                        <input type="checkbox" checked={all()} onChange={(e) => setAll(e.currentTarget.checked)} /> Do this for every conflict
                    </label>
                </div>
            </ModalBody>
            <ModalFooter>
                <Button ref={first} class="files-button" onClick={() => props.onResolve("keep_both", all())}>
                    Keep both
                </Button>
                <Button class="files-button" onClick={() => props.onResolve("skip", all())}>
                    Skip
                </Button>
                <Button tone="danger" class="files-button files-button-danger" onClick={() => props.onResolve("replace", all())}>
                    Replace
                </Button>
                <Button class="files-button" data-modal-dismiss onClick={props.onCancel}>
                    Stop
                </Button>
            </ModalFooter>
        </Modal>
    );
}

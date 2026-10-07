// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The picker behind the editor's "Open from remote…": choose a host
// (open-from-remote.ts), then type a path on it.

import { computeConnColorNum } from "@/app/block/blockutil";
import { TypeAheadModal } from "@/app/modals/typeaheadmodal";
import { atoms, pushNotification } from "@/app/store/global";
import { refreshRemotes, remotesList } from "@/app/store/remotes-store";
import * as keyutil from "@/util/keyutil";
import { createMemo, createSignal, onMount, Show, type JSX } from "solid-js";
import { editorHostScopes } from "./open-from-remote";

export interface OpenFromRemoteModalProps {
    blockRef: { current: HTMLDivElement };
    anchorRef: { current: HTMLElement };
    /** The editor's own host, if it is on one: it is marked in the list. */
    current: string | undefined;
    onOpen: (host: string, path: string) => Promise<void>;
    onClose: () => void;
}

export function OpenFromRemoteModal(props: OpenFromRemoteModalProps): JSX.Element {
    const remotes = remotesList();
    const [host, setHost] = createSignal<string | null>(null);
    const [typed, setTyped] = createSignal("");
    const [path, setPath] = createSignal("~/");
    const [row, setRow] = createSignal(0);
    onMount(() => void refreshRemotes());

    const scopes = createMemo(() => {
        const status = new Map(atoms.allConnStatus().map((c) => [c.connection, c]));
        return editorHostScopes(remotes(), typed(), props.current, (name) => computeConnColorNum(status.get(name)));
    });
    const items = () => scopes().flatMap((s) => s.items);

    const choose = (value: string) => {
        if (!value) return;
        setHost(value);
        setRow(0);
    };
    const open = () => {
        const h = host();
        if (!h) return;
        props.onOpen(h, path()).then(props.onClose, (e) =>
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: `Couldn't open the file on ${h}`,
                message: e instanceof Error ? e.message : String(e),
                timestamp: new Date().toISOString(),
                type: "error",
                expiration: Date.now() + 8000,
            })
        );
    };

    const onKeyDown = (e: MuxKeyboardEvent): boolean => {
        if (keyutil.checkKeyPressed(e, "Escape")) {
            // From the path, back to the hosts; from the hosts, close.
            if (host()) setHost(null);
            else props.onClose();
            return true;
        }
        if (keyutil.checkKeyPressed(e, "Enter")) {
            if (host()) open();
            else choose(items()[row()]?.value ?? "");
            return true;
        }
        if (host()) return false;
        if (keyutil.checkKeyPressed(e, "ArrowUp")) {
            setRow((i) => Math.max(i - 1, 0));
            return true;
        }
        if (keyutil.checkKeyPressed(e, "ArrowDown")) {
            setRow((i) => Math.min(i + 1, items().length - 1));
            return true;
        }
        setRow(0);
        return false;
    };

    return (
        <Show
            when={host()}
            fallback={
                <TypeAheadModal
                    blockRef={props.blockRef}
                    anchorRef={props.anchorRef}
                    suggestions={scopes()}
                    selectIndex={row()}
                    value={typed()}
                    onChange={setTyped}
                    onSelect={choose}
                    onKeyDown={(e) => keyutil.keydownWrapper(onKeyDown)(e)}
                    label="Open a file on (host or user@host)..."
                    autoFocus
                    onClickBackdrop={props.onClose}
                />
            }
        >
            {(h) => (
                <TypeAheadModal
                    blockRef={props.blockRef}
                    anchorRef={props.anchorRef}
                    value={path()}
                    onChange={setPath}
                    onKeyDown={(e) => keyutil.keydownWrapper(onKeyDown)(e)}
                    label={`Path of a file on ${h()}, then Enter`}
                    autoFocus
                    onClickBackdrop={props.onClose}
                />
            )}
        </Show>
    );
}

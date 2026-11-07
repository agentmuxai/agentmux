// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { computeConnColorNum } from "@/app/block/blockutil";
import { atoms, getConnStatusAtom, getHostName, getUserName, MOS } from "@/app/store/global";
import { globalRefocusWithTimeout } from "@/app/store/keymodel";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { NodeModel } from "@/layout/index";
import * as keyutil from "@/util/keyutil";
import { createEffect, createSignal, type Accessor } from "solid-js";
import { setBlockMeta } from "@/app/store/block-meta";
import { refreshRemotes, remotesList } from "@/app/store/remotes-store";
import { remoteSuggestionScopes } from "./conn-remote-items";
import {
    createFilteredLocalSuggestionItem,
    getDisconnectItem,
    getManageRemotesItem,
    getNewConnectionSuggestionItem,
    getReconnectItem,
    getS3Suggestions,
} from "./conntypeahead-items";
import type { ConnStatus } from "@/types/rpc/ConnStatus";

export interface ConnModalProps {
    blockId: string;
    viewModel: ViewModel;
    blockRef: { current: HTMLDivElement };
    connBtnRef: { current: HTMLDivElement };
    changeConnModalOpen: Accessor<boolean>;
    setChangeConnModalOpen: (v: boolean) => void;
    nodeModel: NodeModel;
}

/** The block, the connections, the remotes and the typed text the picker works from. */
function createConnData(props: ConnModalProps) {
    const { blockId, viewModel, nodeModel } = props;
    const [connSelected, setConnSelected] = createSignal("");
    const [blockData] = MOS.useMuxObjectValue<Block>(MOS.makeORef("block", blockId));
    const isNodeFocused = nodeModel.isFocused;
    const connection = () => blockData()?.meta?.connection;
    const connStatus = () => getConnStatusAtom(connection())();
    const remotes = remotesList();
    const [s3List, setS3List] = createSignal<Array<string>>([]);
    const allConnStatus = atoms.allConnStatus;
    const [rowIndex, setRowIndex] = createSignal(0);
    const fullConfig = () => atoms.fullConfigAtom();
    const showS3 = () => (viewModel.showS3 ? viewModel.showS3() : false);

    return { connSelected, setConnSelected, blockData, isNodeFocused, connection, connStatus, remotes, s3List, setS3List, allConnStatus, rowIndex, setRowIndex, fullConfig, showS3 };
}

type ConnData = ReturnType<typeof createConnData>;

/** Reloads the remotes and the S3 profiles whenever the picker is opened. */
function loadListsWhenOpen(props: ConnModalProps, setS3List: (v: Array<string>) => void): void {
    const { changeConnModalOpen } = props;
    createEffect(() => {
        if (!changeConnModalOpen()) {
            return;
        }
        void refreshRemotes();
        RpcApi.ConnListAWSCommand(TabRpcClient, { timeout: 2000 })
            .then((s3ListResult) => setS3List(s3ListResult ?? []))
            .catch((e) => console.log("unable to load s3 list from backend:", e));
    });
}

/** Points the pane at another connection (the local one for an empty name), moving its file to match. */
function createChangeConnection(blockId: string, blockData: ConnData["blockData"]) {
    const changeConnection = async (connName: string) => {
        if (connName == "") {
            connName = null;
        }
        if (connName == blockData()?.meta?.connection) {
            return;
        }
        const isAws = connName?.startsWith("aws:");
        const oldFile = blockData()?.meta?.file ?? "";
        let newFile: string;
        if (oldFile == "") {
            newFile = "";
        } else if (isAws) {
            newFile = "/";
        } else {
            newFile = "~";
        }
        await setBlockMeta(blockId, { connection: connName, file: newFile, "cmd:cwd": null });
        try {
            await RpcApi.ConnEnsureCommand(
                TabRpcClient,
                { connname: connName, logblockid: blockId },
                { timeout: 60000 }
            );
        } catch (e) {
            console.log("error connecting", blockId, connName, e);
        }
    };

    return changeConnection;
}

/** The picker's suggestions for what is typed: local, the remotes, S3, disconnect, manage remotes and a new connection. */
function createSuggestions(
    props: ConnModalProps,
    data: ConnData,
    changeConnection: (connName: string) => Promise<void>,
) {
    const { blockId, setChangeConnModalOpen } = props;
    const { allConnStatus, connStatus, connection, fullConfig, connSelected, remotes, showS3, s3List } = data;

    const suggestions = () => {
        const connStatusMap = new Map<string, ConnStatus>();
        let maxActiveConnNum = 1;
        for (const conn of allConnStatus()) {
            if (conn.activeconnnum > maxActiveConnNum) {
                maxActiveConnNum = conn.activeconnnum;
            }
            connStatusMap.set(conn.connection, conn);
        }

        const cs = connStatus();
        const conn = connection();
        const fc = fullConfig();

        const reconnectSuggestionItem = getReconnectItem(cs, connSelected(), blockId);
        const localName = getUserName() + "@" + getHostName();
        const localItems = createFilteredLocalSuggestionItem(localName, conn, connSelected());
        const localSuggestions: SuggestionConnectionScope | null = localItems.length
            ? { headerText: "Local", items: localItems }
            : null;
        // Pinned, SSH hosts, Recent and WSL, as in Connectors > Remotes (§4.7).
        const remoteScopes = remoteSuggestionScopes(remotes(), connSelected(), conn, (name) =>
            computeConnColorNum(connStatusMap.get(name))
        );
        // Only the remotes the picker shows: a hidden one left out must still
        // offer "(New Connection)" for what was typed.
        const remoteNames = remoteScopes.flatMap((s) => s.items.map((i) => i.value));
        let s3Suggestions: SuggestionConnectionScope = null;
        if (showS3()) {
            s3Suggestions = getS3Suggestions(
                s3List(),
                conn,
                connSelected(),
                connStatusMap,
                fc
            );
        }
        const manageRemotesItem = getManageRemotesItem(blockId, conn, setChangeConnModalOpen, connSelected());
        const disconnectItem = getDisconnectItem(conn, connStatusMap);
        const newConnectionSuggestionItem = getNewConnectionSuggestionItem(
            connSelected(),
            localName,
            remoteNames,
            [],
            s3List(),
            changeConnection,
            setChangeConnModalOpen
        );

        const sug: Array<SuggestionsType> = [
            ...(reconnectSuggestionItem ? [reconnectSuggestionItem] : []),
            ...(localSuggestions ? [localSuggestions] : []),
            ...remoteScopes,
            ...(s3Suggestions ? [s3Suggestions] : []),
            ...(disconnectItem ? [disconnectItem] : []),
            ...(manageRemotesItem ? [manageRemotesItem] : []),
            ...(newConnectionSuggestionItem ? [newConnectionSuggestionItem] : []),
        ];
        return sug;
    };

    return suggestions;
}

/** The highlighted row, the arrow, Enter and Escape keys, and the clamp when the list shrinks. */
function createKeyboard(
    props: ConnModalProps,
    data: ConnData,
    suggestions: () => Array<SuggestionsType>,
    changeConnection: (connName: string) => Promise<void>,
) {
    const { setChangeConnModalOpen } = props;
    const { setConnSelected, rowIndex, setRowIndex } = data;

    const selectionList = () => {
        let list: Array<SuggestionConnectionItem> = suggestions().flatMap((item) => {
            if ("items" in item) {
                return item.items;
            }
            return item;
        });

        // quick way to change icon color when highlighted
        list = list.map((item, index) => {
            if (index == rowIndex() && item.iconColor == "var(--grey-text-color)") {
                item.iconColor = "var(--main-text-color)";
            }
            return item;
        });
        return list;
    };

    const handleTypeAheadKeyDown = (muxEvent: MuxKeyboardEvent): boolean => {
        const sl = selectionList();
        if (keyutil.checkKeyPressed(muxEvent, "Enter")) {
            const rowItem = sl[rowIndex()];
            if ("onSelect" in rowItem && rowItem.onSelect) {
                rowItem.onSelect(rowItem.value);
            } else {
                changeConnection(rowItem.value);
                setChangeConnModalOpen(false);
                globalRefocusWithTimeout(10);
            }
            setRowIndex(0);
            return true;
        }
        if (keyutil.checkKeyPressed(muxEvent, "Escape")) {
            setChangeConnModalOpen(false);
            setConnSelected("");
            globalRefocusWithTimeout(10);
            return true;
        }
        if (keyutil.checkKeyPressed(muxEvent, "ArrowUp")) {
            setRowIndex((idx) => Math.max(idx - 1, 0));
            return true;
        }
        if (keyutil.checkKeyPressed(muxEvent, "ArrowDown")) {
            setRowIndex((idx) => Math.min(idx + 1, sl.length - 1));
            return true;
        }
        setRowIndex(0);
        return false;
    };

    // Clamp rowIndex when list shrinks
    createEffect(() => {
        const sl = selectionList();
        setRowIndex((idx) => Math.min(idx, sl.flat().length - 1));
    });

    return { handleTypeAheadKeyDown };
}

/** The state of the "Connect to" picker of one pane. */
export function createConnPicker(props: ConnModalProps) {
    const data = createConnData(props);
    loadListsWhenOpen(props, data.setS3List);
    const changeConnection = createChangeConnection(props.blockId, data.blockData);
    const suggestions = createSuggestions(props, data, changeConnection);
    const { handleTypeAheadKeyDown } = createKeyboard(props, data, suggestions, changeConnection);

    return { ...data, changeConnection, suggestions, handleTypeAheadKeyDown };
}

export type ConnPickerState = ReturnType<typeof createConnPicker>;

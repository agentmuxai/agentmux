// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { computeConnColorNum } from "@/app/block/blockutil";
import { TypeAheadModal } from "@/app/modals/typeaheadmodal";
import {
    atoms,
    getConnStatusAtom,
    getHostName,
    getUserName,
    MOS,
} from "@/app/store/global";
import { globalRefocusWithTimeout } from "@/app/store/keymodel";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { NodeModel } from "@/layout/index";
import * as keyutil from "@/util/keyutil";
import { createEffect, createSignal, onMount, type Accessor } from "solid-js";
import { setBlockMeta } from "@/app/store/block-meta";
import { refreshRemotes, remotesList } from "@/app/store/remotes-store";
import { openRemotesInPane } from "@/app/view/remotes/open-remotes";
import { remoteSuggestionScopes } from "./conn-remote-items";

// newConnList -> connList => filteredList -> remoteItems -> sortedRemoteItems => remoteSuggestion
// filteredList -> createNew

function filterConnections(
    connList: Array<string>,
    connSelected: string,
    fullConfig: FullConfigType
): Array<string> {
    const connectionsConfig = fullConfig.connections;
    return connList.filter((conn) => {
        const hidden = connectionsConfig?.[conn]?.["display:hidden"] ?? false;
        return conn.includes(connSelected) && !hidden;
    });
}

function sortConnSuggestionItems(
    connSuggestions: Array<SuggestionConnectionItem>,
    fullConfig: FullConfigType
): Array<SuggestionConnectionItem> {
    const connectionsConfig = fullConfig.connections;
    return connSuggestions.sort((itemA: SuggestionConnectionItem, itemB: SuggestionConnectionItem) => {
        const connNameA = itemA.value;
        const connNameB = itemB.value;
        const valueA = connectionsConfig?.[connNameA]?.["display:order"] ?? 0;
        const valueB = connectionsConfig?.[connNameB]?.["display:order"] ?? 0;
        return valueA - valueB;
    });
}

function createFilteredLocalSuggestionItem(
    localName: string,
    connection: string,
    connSelected: string
): Array<SuggestionConnectionItem> {
    if (localName.includes(connSelected)) {
        const localSuggestion: SuggestionConnectionItem = {
            status: "connected",
            icon: "laptop",
            iconColor: "var(--grey-text-color)",
            value: "",
            label: localName,
            current: connection == null,
        };
        return [localSuggestion];
    }
    return [];
}

function createS3SuggestionItems(
    s3Profiles: Array<string>,
    connStatusMap: Map<string, ConnStatus>,
    connection: string
): Array<SuggestionConnectionItem> {
    return s3Profiles.map((profileName) => {
        const connStatus = connStatusMap.get(profileName);
        const item: SuggestionConnectionItem = {
            status: "connected",
            icon: "database",
            iconColor: "var(--accent-color)",
            value: profileName,
            label: profileName,
            current: profileName == connection,
        };
        return item;
    });
}

function getReconnectItem(
    connStatus: ConnStatus,
    connSelected: string,
    blockId: string
): SuggestionConnectionItem | null {
    if (connSelected != "" || (connStatus.status != "disconnected" && connStatus.status != "error")) {
        return null;
    }
    const reconnectSuggestionItem: SuggestionConnectionItem = {
        status: "connected",
        icon: "arrow-right-arrow-left",
        iconColor: "var(--grey-text-color)",
        label: `Reconnect to ${connStatus.connection}`,
        value: "",
        onSelect: async (_: string) => {
            const prtn = RpcApi.ConnConnectCommand(
                TabRpcClient,
                { host: connStatus.connection, logblockid: blockId },
                { timeout: 60000 }
            );
            prtn.catch((e) => console.log("error reconnecting", connStatus.connection, e));
        },
    };
    return reconnectSuggestionItem;
}

function getS3Suggestions(
    s3Profiles: Array<string>,
    connection: string,
    connSelected: string,
    connStatusMap: Map<string, ConnStatus>,
    fullConfig: FullConfigType
): SuggestionConnectionScope | null {
    const filtered = filterConnections(s3Profiles, connSelected, fullConfig);
    const s3Items = createS3SuggestionItems(filtered, connStatusMap, connection);
    const sortedS3Items = sortConnSuggestionItems(s3Items, fullConfig);
    if (sortedS3Items.length == 0) {
        return null;
    }
    const s3Suggestions: SuggestionConnectionScope = {
        headerText: "S3",
        items: sortedS3Items,
    };
    return s3Suggestions;
}

function getDisconnectItem(
    connection: string,
    connStatusMap: Map<string, ConnStatus>
): SuggestionConnectionItem | null {
    if (!connection) {
        return null;
    }
    const connStatus = connStatusMap.get(connection);
    if (!connStatus || connStatus.status != "connected") {
        return null;
    }
    const disconnectSuggestionItem: SuggestionConnectionItem = {
        status: "connected",
        icon: "xmark",
        iconColor: "var(--grey-text-color)",
        label: `Disconnect ${connStatus.connection}`,
        value: "",
        onSelect: async (_: string) => {
            const prtn = RpcApi.ConnDisconnectCommand(TabRpcClient, connection, { timeout: 60000 });
            prtn.catch((e) => console.log("error disconnecting", connStatus.connection, e));
        },
    };
    return disconnectSuggestionItem;
}

/** "Manage remotes…": the Remotes pane, in this pane (SPEC_REMOTES_PANE_2026_10_05.md §4.7). */
function getManageRemotesItem(
    blockId: string,
    connection: string | undefined,
    setChangeConnModalOpen: (v: boolean) => void,
    connSelected: string
): SuggestionConnectionItem | null {
    if (connSelected != "") {
        return null;
    }
    return {
        status: "disconnected",
        icon: "server",
        iconColor: "var(--grey-text-color)",
        value: "Manage remotes…",
        label: "Manage remotes…",
        onSelect: () => {
            setChangeConnModalOpen(false);
            openRemotesInPane(blockId, connection || undefined).catch((e) => console.log("could not open Remotes", e));
        },
    };
}

function getNewConnectionSuggestionItem(
    connSelected: string,
    localName: string,
    remoteConns: Array<string>,
    wslConns: Array<string>,
    s3Conns: Array<string>,
    changeConnection: (connName: string) => Promise<void>,
    setChangeConnModalOpen: (v: boolean) => void
): SuggestionConnectionItem | null {
    const allCons = ["", localName, ...remoteConns, ...wslConns, ...s3Conns];
    if (allCons.includes(connSelected)) {
        return null;
    }
    const newConnectionSuggestion: SuggestionConnectionItem = {
        status: "connected",
        icon: "plus",
        iconColor: "var(--grey-text-color)",
        label: `${connSelected} (New Connection)`,
        value: "",
        onSelect: (_: string) => {
            changeConnection(connSelected);
            setChangeConnModalOpen(false);
        },
    };
    return newConnectionSuggestion;
}

const ChangeConnectionBlockModal = ({
    blockId,
    viewModel,
    blockRef,
    connBtnRef,
    changeConnModalOpen,
    setChangeConnModalOpen,
    nodeModel,
}: {
    blockId: string;
    viewModel: ViewModel;
    blockRef: { current: HTMLDivElement };
    connBtnRef: { current: HTMLDivElement };
    changeConnModalOpen: Accessor<boolean>;
    setChangeConnModalOpen: (v: boolean) => void;
    nodeModel: NodeModel;
}) => {
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

    createEffect(() => {
        if (!changeConnModalOpen()) {
            return;
        }
        void refreshRemotes();
        RpcApi.ConnListAWSCommand(TabRpcClient, { timeout: 2000 })
            .then((s3ListResult) => setS3List(s3ListResult ?? []))
            .catch((e) => console.log("unable to load s3 list from backend:", e));
    });

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

        const rtInfo = { "cmd:hascurcwd": null };
        const rtInfoData: CommandSetRTInfoData = {
            oref: MOS.makeORef("block", blockId),
            data: rtInfo,
        };
        RpcApi.SetRTInfoCommand(TabRpcClient, rtInfoData).catch((e) =>
            console.log("error setting RT info", e)
        );
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
        // Pinned, SSH hosts, Recent and WSL, as in the Remotes pane (§4.7).
        const remoteScopes = remoteSuggestionScopes(remotes(), connSelected(), conn, (name) =>
            computeConnColorNum(connStatusMap.get(name))
        );
        const remoteNames = remotes().map((r) => r.name);
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

    if (!changeConnModalOpen()) {
        return null;
    }

    return (
        <TypeAheadModal
            blockRef={blockRef}
            anchorRef={connBtnRef}
            suggestions={suggestions()}
            onSelect={(selected: string) => {
                changeConnection(selected);
                setChangeConnModalOpen(false);
                globalRefocusWithTimeout(10);
            }}
            selectIndex={rowIndex()}
            autoFocus={isNodeFocused()}
            onKeyDown={(e) => keyutil.keydownWrapper(handleTypeAheadKeyDown)(e)}
            onChange={(current: string) => setConnSelected(current)}
            value={connSelected()}
            label="Connect to (username@host)..."
            onClickBackdrop={() => setChangeConnModalOpen(false)}
        />
    );
};

export { ChangeConnectionBlockModal };

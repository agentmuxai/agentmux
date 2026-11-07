// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { openRemotesInPane } from "@/app/view/remotes/open-remotes";
import type { ConnStatus } from "@/types/rpc/ConnStatus";

// newConnList -> connList => filteredList -> remoteItems -> sortedRemoteItems => remoteSuggestion
// filteredList -> createNew

export function filterConnections(
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

export function sortConnSuggestionItems(
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

export function createFilteredLocalSuggestionItem(
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

export function createS3SuggestionItems(
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

export function getReconnectItem(
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

export function getS3Suggestions(
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

export function getDisconnectItem(
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

/** "Manage remotes…": Connectors → Remotes, in this pane (SPEC_REMOTES_PANE_2026_10_05.md §4.7). */
export function getManageRemotesItem(
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

export function getNewConnectionSuggestionItem(
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

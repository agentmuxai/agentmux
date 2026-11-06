// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What McpManager's controls do: open and close the cards, start a form, ask before a delete, and the
 * right-click menu. Plain functions over the state; nothing here creates a computation.
 */

import { MENU_ICON } from "@/app/util/menu-icons";
import type { McpServerCatalogItem } from "@/types/rpc/McpServerCatalogItem";
import type { McpPreloadEntry } from "./mcp-preload-catalog";
import type { McpState } from "./mcp-manager-state";

export function createMcpActions(s: McpState) {
    const { model } = s;

    const closeCatalog = () => {
        model.setError(null);
        model.setDraft(null);
        model.setSelectedId(null);
    };
    const toggleServer = (server: McpServerCatalogItem) => {
        s.setObservedOpen(null);
        if (s.catalogOpen() === server.id) closeCatalog();
        else model.handleSelect(server);
    };
    const toggleObserved = (key: string) => {
        if (s.observedOpen() === key) {
            s.setObservedOpen(null);
            return;
        }
        closeCatalog();
        s.setObservedOpen(key);
    };
    const startNew = () => {
        s.setObservedOpen(null);
        model.startNew();
    };
    // A preset already in the catalog opens its card instead of a second copy.
    const pickPreset = (entry: McpPreloadEntry) => {
        s.setObservedOpen(null);
        const existing = model.serversAtom().find((x) => x.name === entry.name);
        if (existing) {
            s.setFilter("");
            model.handleSelect(existing);
        } else {
            model.startFromCatalog(entry);
        }
    };
    const requestDelete = (server: McpServerCatalogItem) => {
        s.setDeleting(server);
        s.setDeletingBound(null);
        void model.boundAgentNames(server).then((names) => {
            if (s.deleting()?.id === server.id) s.setDeletingBound(names);
        });
    };

    const serverMenu = (server: McpServerCatalogItem): ContextMenuItem[] => {
        const agents = model.agentsAtom();
        return [
            {
                label: "Edit",
                icon: MENU_ICON.rename,
                click: () => {
                    s.setObservedOpen(null);
                    model.startEdit(server);
                },
            },
            agents.length === 0
                ? { label: "Bind to agent", icon: MENU_ICON.bind, sublabel: "no agents", enabled: false }
                : {
                      label: "Bind to agent",
                      icon: MENU_ICON.bind,
                      type: "submenu",
                      submenu: agents.map((a) => ({
                          label: a.name,
                          click: () => void model.bindToAgent(server.id, a.id),
                      })),
                  },
            // Last, after a separator, so it is never under the pointer.
            { type: "separator" },
            { label: "Delete…", icon: MENU_ICON.delete, click: () => requestDelete(server) },
        ];
    };

    return { closeCatalog, toggleServer, toggleObserved, startNew, pickPreset, requestDelete, serverMenu };
}

export type McpActions = ReturnType<typeof createMcpActions>;

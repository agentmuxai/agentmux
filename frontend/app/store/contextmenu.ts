// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi, globalStore } from "./global";

class ContextMenuModelType {
    handlers: Map<string, () => void> = new Map(); // id -> handler

    constructor() {}

    // Must be called from wave.ts:initBare() after setupTauriApi() has installed window.api.
    // Calling getApi() here (module level) would crash before window.api exists.
    init() {
        getApi().onContextMenuClick(this.handleContextMenuClick.bind(this));
    }

    handleContextMenuClick(id: string): void {
        const handler = this.handlers.get(id);
        if (handler) {
            handler();
        }
    }

    _convertAndRegisterMenu(menu: ContextMenuItem[]): NativeContextMenuItem[] {
        const nativeMenuItems: NativeContextMenuItem[] = [];
        for (const item of menu) {
            const nativeItem: NativeContextMenuItem = {
                role: item.role,
                type: item.type,
                label: item.label,
                sublabel: item.sublabel,
                id: crypto.randomUUID(),
                checked: item.checked,
            };
            if (item.visible === false) {
                nativeItem.visible = false;
            }
            if (item.enabled === false) {
                nativeItem.enabled = false;
            }
            if (item.click) {
                this.handlers.set(nativeItem.id, item.click);
            }
            if (item.submenu) {
                nativeItem.submenu = this._convertAndRegisterMenu(item.submenu);
            }
            nativeMenuItems.push(nativeItem);
        }
        return nativeMenuItems;
    }

    showContextMenu(menu: ContextMenuItem[], ev: MouseEvent | { stopPropagation(): void }): void {
        ev.stopPropagation();
        this.handlers.clear();
        const nativeMenuItems = this._convertAndRegisterMenu(menu);
        const position = { x: Math.round((ev as MouseEvent).clientX), y: Math.round((ev as MouseEvent).clientY) };
        getApi().showContextMenu(atoms.workspace()?.oid, nativeMenuItems, position);
    }
}

const ContextMenuModel = new ContextMenuModelType();

export { ContextMenuModel, ContextMenuModelType };

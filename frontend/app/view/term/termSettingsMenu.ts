// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { atoms, getSettingsKeyAtom } from "@/store/global";
import { showHostSessions } from "./hostSessions";
import type { TermViewModel } from "./termViewModel";

/** A connection name that is an SSH destination: not local, not WSL. */
export function isSshConnection(conn: unknown): boolean {
    return typeof conn === "string" && conn.trim() !== "" && conn.trim() !== "local" && !conn.startsWith("wsl://");
}

export function buildSettingsMenuItems(model: TermViewModel): ContextMenuItem[] {
    const fullConfig = atoms.fullConfigAtom();
    const termThemes = fullConfig?.termthemes ?? {};
    const termThemeKeys = Object.keys(termThemes);
    const curThemeName = model.meta()?.["term:theme"];
    const defaultFontSize = getSettingsKeyAtom("term:fontsize")() ?? 15;
    const transparencyMeta = model.meta()?.["term:transparency"];
    const meta = model.meta();
    const overrideFontSize = meta?.["term:fontsize"];

    termThemeKeys.sort((a, b) => {
        return (termThemes[a]["display:order"] ?? 0) - (termThemes[b]["display:order"] ?? 0);
    });
    const fullMenu: ContextMenuItem[] = [];

    // Theme submenu
    const submenu: ContextMenuItem[] = termThemeKeys.map((themeName) => {
        return {
            label: termThemes[themeName]["display:name"] ?? themeName,
            type: "checkbox",
            checked: curThemeName == themeName,
            click: () => model.setTerminalTheme(themeName),
        };
    });
    submenu.unshift({
        label: "Default",
        type: "checkbox",
        checked: curThemeName == null,
        click: () => model.setTerminalTheme(null),
    });

    // Transparency submenu
    const transparencySubMenu: ContextMenuItem[] = [];
    transparencySubMenu.push({
        label: "Default",
        type: "checkbox",
        checked: transparencyMeta == null,
        click: () => {
            void model.setMeta({ "term:transparency": null });
        },
    });
    transparencySubMenu.push({
        label: "Transparent Background",
        type: "checkbox",
        checked: transparencyMeta == 0.5,
        click: () => {
            void model.setMeta({ "term:transparency": 0.5 });
        },
    });
    transparencySubMenu.push({
        label: "No Transparency",
        type: "checkbox",
        checked: transparencyMeta == 0,
        click: () => {
            void model.setMeta({ "term:transparency": 0 });
        },
    });

    // Font size submenu
    const fontSizeSubMenu: ContextMenuItem[] = [6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18].map(
        (fontSize: number) => {
            return {
                label: fontSize.toString() + "px",
                type: "checkbox",
                checked: overrideFontSize == fontSize,
                click: () => {
                    void model.setMeta({ "term:fontsize": fontSize });
                },
            };
        }
    );
    fontSizeSubMenu.unshift({
        label: "Default (" + defaultFontSize + "px)",
        type: "checkbox",
        checked: overrideFontSize == null,
        click: () => {
            void model.setMeta({ "term:fontsize": null });
        },
    });

    // Terminal Zoom submenu
    const currentZoom = meta?.["term:zoom"] ?? 1.0;
    const zoomLevels = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
    const zoomSubMenu: ContextMenuItem[] = zoomLevels.map((zoom: number) => {
        const percentage = Math.round(zoom * 100);
        return {
            label: `${percentage}%`,
            type: "checkbox",
            checked: Math.abs(currentZoom - zoom) < 0.01,
            click: () => {
                void model.setMeta({ "term:zoom": zoom === 1.0 ? null : zoom });
            },
        };
    });
    zoomSubMenu.push({ type: "separator" });
    zoomSubMenu.push({
        label: "Reset to Default",
        click: () => {
            void model.setMeta({ "term:zoom": null });
        },
    });

    fullMenu.push({ label: "Themes", submenu: submenu });
    fullMenu.push({ label: "Font Size", submenu: fontSizeSubMenu });
    fullMenu.push({ label: "Terminal Zoom", submenu: zoomSubMenu });
    fullMenu.push({ label: "Transparency", submenu: transparencySubMenu });
    fullMenu.push({ type: "separator" });
    fullMenu.push({
        label: "Force Restart Controller",
        click: model.forceRestartController.bind(model),
    });
    // An SSH pane can keep its shell running on the host (AgentMux's helper
    // there holds it): a dropped link, sleep or a restart reattaches instead
    // of ending it. Switching restarts the pane's connection.
    if (isSshConnection(meta?.connection)) {
        const durable = meta?.["term:durable"] === true;
        fullMenu.push({
            label: "Keep Session Alive (durable)",
            type: "checkbox",
            checked: durable,
            click: () => {
                // Off: forget the session too; srv ends it on the restart
                // (a pane that is no longer durable never comes back to it).
                const patch: MetaType = durable
                    ? { "term:durable": null, "remote:session_id": null }
                    : { "term:durable": true };
                void model.setMeta(patch).then(() => model.forceRestartController());
            },
        });
        // The host's durable sessions, including any no pane holds any more.
        const conn = String(meta.connection).trim();
        fullMenu.push({
            label: `Sessions on ${conn}…`,
            click: () => void showHostSessions(conn, model.blockId),
        });
    }

    const isClearOnStart = meta?.["cmd:clearonstart"];
    fullMenu.push({
        label: "Clear Output On Restart",
        submenu: [
            {
                label: "On",
                type: "checkbox",
                checked: isClearOnStart,
                click: () => {
                    void model.setMeta({ "cmd:clearonstart": true });
                },
            },
            {
                label: "Off",
                type: "checkbox",
                checked: !isClearOnStart,
                click: () => {
                    void model.setMeta({ "cmd:clearonstart": false });
                },
            },
        ],
    });

    const runOnStart = meta?.["cmd:runonstart"];
    fullMenu.push({
        label: "Run On Startup",
        submenu: [
            {
                label: "On",
                type: "checkbox",
                checked: runOnStart,
                click: () => {
                    void model.setMeta({ "cmd:runonstart": true });
                },
            },
            {
                label: "Off",
                type: "checkbox",
                checked: !runOnStart,
                click: () => {
                    void model.setMeta({ "cmd:runonstart": false });
                },
            },
        ],
    });

    if (meta?.["term:vdomtoolbarblockid"]) {
        fullMenu.push({ type: "separator" });
        fullMenu.push({
            label: "Close Toolbar",
            click: () => {
                RpcApi.DeleteSubBlockCommand(TabRpcClient, { blockid: meta["term:vdomtoolbarblockid"] });
            },
        });
    }

    const debugConn = meta?.["term:conndebug"];
    fullMenu.push({
        label: "Debug Connection",
        submenu: [
            {
                label: "Off",
                type: "checkbox",
                checked: !debugConn,
                click: () => {
                    void model.setMeta({ "term:conndebug": null });
                },
            },
            {
                label: "Info",
                type: "checkbox",
                checked: debugConn == "info",
                click: () => {
                    void model.setMeta({ "term:conndebug": "info" });
                },
            },
            {
                label: "Verbose",
                type: "checkbox",
                checked: debugConn == "debug",
                click: () => {
                    void model.setMeta({ "term:conndebug": "debug" });
                },
            },
        ],
    });

    return fullMenu;
}

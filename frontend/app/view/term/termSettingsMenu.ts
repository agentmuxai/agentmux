// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atoms, getBlockMetaKeyAtom, getSettingsKeyAtom, WOS } from "@/store/global";
import type { TermViewModel } from "./termViewModel";

export function buildSettingsMenuItems(model: TermViewModel): ContextMenuItem[] {
    const fullConfig = atoms.fullConfigAtom();
    const termThemes = fullConfig?.termthemes ?? {};
    const termThemeKeys = Object.keys(termThemes);
    const curThemeName = getBlockMetaKeyAtom(model.blockId, "term:theme")();
    const defaultFontSize = getSettingsKeyAtom("term:fontsize")() ?? 12;
    const transparencyMeta = getBlockMetaKeyAtom(model.blockId, "term:transparency")();
    const blockData = model.blockAtom();
    const overrideFontSize = blockData?.meta?.["term:fontsize"];

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
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", model.blockId),
                meta: { "term:transparency": null },
            });
        },
    });
    transparencySubMenu.push({
        label: "Transparent Background",
        type: "checkbox",
        checked: transparencyMeta == 0.5,
        click: () => {
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", model.blockId),
                meta: { "term:transparency": 0.5 },
            });
        },
    });
    transparencySubMenu.push({
        label: "No Transparency",
        type: "checkbox",
        checked: transparencyMeta == 0,
        click: () => {
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", model.blockId),
                meta: { "term:transparency": 0 },
            });
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
                    RpcApi.SetMetaCommand(TabRpcClient, {
                        oref: WOS.makeORef("block", model.blockId),
                        meta: { "term:fontsize": fontSize },
                    });
                },
            };
        }
    );
    fontSizeSubMenu.unshift({
        label: "Default (" + defaultFontSize + "px)",
        type: "checkbox",
        checked: overrideFontSize == null,
        click: () => {
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", model.blockId),
                meta: { "term:fontsize": null },
            });
        },
    });

    // Terminal Zoom submenu
    const currentZoom = blockData?.meta?.["term:zoom"] ?? 1.0;
    const zoomLevels = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
    const zoomSubMenu: ContextMenuItem[] = zoomLevels.map((zoom: number) => {
        const percentage = Math.round(zoom * 100);
        return {
            label: `${percentage}%`,
            type: "checkbox",
            checked: Math.abs(currentZoom - zoom) < 0.01,
            click: () => {
                RpcApi.SetMetaCommand(TabRpcClient, {
                    oref: WOS.makeORef("block", model.blockId),
                    meta: { "term:zoom": zoom === 1.0 ? null : zoom },
                });
            },
        };
    });
    zoomSubMenu.push({ type: "separator" });
    zoomSubMenu.push({
        label: "Reset to Default",
        click: () => {
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", model.blockId),
                meta: { "term:zoom": null },
            });
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

    const isClearOnStart = blockData?.meta?.["cmd:clearonstart"];
    fullMenu.push({
        label: "Clear Output On Restart",
        submenu: [
            {
                label: "On",
                type: "checkbox",
                checked: isClearOnStart,
                click: () => {
                    RpcApi.SetMetaCommand(TabRpcClient, {
                        oref: WOS.makeORef("block", model.blockId),
                        meta: { "cmd:clearonstart": true },
                    });
                },
            },
            {
                label: "Off",
                type: "checkbox",
                checked: !isClearOnStart,
                click: () => {
                    RpcApi.SetMetaCommand(TabRpcClient, {
                        oref: WOS.makeORef("block", model.blockId),
                        meta: { "cmd:clearonstart": false },
                    });
                },
            },
        ],
    });

    const runOnStart = blockData?.meta?.["cmd:runonstart"];
    fullMenu.push({
        label: "Run On Startup",
        submenu: [
            {
                label: "On",
                type: "checkbox",
                checked: runOnStart,
                click: () => {
                    RpcApi.SetMetaCommand(TabRpcClient, {
                        oref: WOS.makeORef("block", model.blockId),
                        meta: { "cmd:runonstart": true },
                    });
                },
            },
            {
                label: "Off",
                type: "checkbox",
                checked: !runOnStart,
                click: () => {
                    RpcApi.SetMetaCommand(TabRpcClient, {
                        oref: WOS.makeORef("block", model.blockId),
                        meta: { "cmd:runonstart": false },
                    });
                },
            },
        ],
    });

    if (blockData?.meta?.["term:vdomtoolbarblockid"]) {
        fullMenu.push({ type: "separator" });
        fullMenu.push({
            label: "Close Toolbar",
            click: () => {
                RpcApi.DeleteSubBlockCommand(TabRpcClient, { blockid: blockData.meta["term:vdomtoolbarblockid"] });
            },
        });
    }

    const debugConn = blockData?.meta?.["term:conndebug"];
    fullMenu.push({
        label: "Debug Connection",
        submenu: [
            {
                label: "Off",
                type: "checkbox",
                checked: !debugConn,
                click: () => {
                    RpcApi.SetMetaCommand(TabRpcClient, {
                        oref: WOS.makeORef("block", model.blockId),
                        meta: { "term:conndebug": null },
                    });
                },
            },
            {
                label: "Info",
                type: "checkbox",
                checked: debugConn == "info",
                click: () => {
                    RpcApi.SetMetaCommand(TabRpcClient, {
                        oref: WOS.makeORef("block", model.blockId),
                        meta: { "term:conndebug": "info" },
                    });
                },
            },
            {
                label: "Verbose",
                type: "checkbox",
                checked: debugConn == "debug",
                click: () => {
                    RpcApi.SetMetaCommand(TabRpcClient, {
                        oref: WOS.makeORef("block", model.blockId),
                        meta: { "term:conndebug": "debug" },
                    });
                },
            },
        ],
    });

    return fullMenu;
}

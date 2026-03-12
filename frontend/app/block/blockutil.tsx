// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { NumActiveConnColors } from "@/app/block/blockframe";
import { getConnStatusAtom } from "@/app/store/global";
import * as util from "@/util/util";
import clsx from "clsx";
import type { JSX } from "solid-js";
import { createMemo, createSignal } from "solid-js";
import dotsUrl from "../asset/dots-anim-4.svg?url";

export const colorRegex = /^((#[0-9a-f]{6,8})|([a-z]+))$/;

export function blockViewToIcon(view: string): string {
    if (view == "term") {
        return "terminal";
    }
    if (view == "help") {
        return "circle-question";
    }
    return "square";
}

export function blockViewToName(view: string): string {
    if (util.isBlank(view)) {
        return "(No View)";
    }
    if (view == "term") {
        return "Terminal";
    }
    if (view == "help") {
        return "Help";
    }
    return view;
}

export function processTitleString(titleString: string): JSX.Element[] {
    if (titleString == null) {
        return null;
    }
    const tagRegex = /<(\/)?([a-z]+)(?::([#a-z0-9@-]+))?>/g;
    let lastIdx = 0;
    let match;
    let partsStack: any[][] = [[]];
    while ((match = tagRegex.exec(titleString)) != null) {
        const lastPart = partsStack[partsStack.length - 1];
        const before = titleString.substring(lastIdx, match.index);
        lastPart.push(before);
        lastIdx = match.index + match[0].length;
        const [_, isClosing, tagName, tagParam] = match;
        if (tagName == "icon" && !isClosing) {
            if (tagParam == null) {
                continue;
            }
            const iconClass = util.makeIconClass(tagParam, false);
            if (iconClass == null) {
                continue;
            }
            lastPart.push(<i class={iconClass} />);
            continue;
        }
        if (tagName == "c" || tagName == "color") {
            if (isClosing) {
                if (partsStack.length <= 1) {
                    continue;
                }
                partsStack.pop();
                continue;
            }
            if (tagParam == null) {
                continue;
            }
            if (!tagParam.match(colorRegex)) {
                continue;
            }
            let children: any[] = [];
            const rtag = <span style={{ color: tagParam }}>{children}</span>;
            lastPart.push(rtag);
            partsStack.push(children);
            continue;
        }
        if (tagName == "i" || tagName == "b") {
            if (isClosing) {
                if (partsStack.length <= 1) {
                    continue;
                }
                partsStack.pop();
                continue;
            }
            let children: any[] = [];
            // Use dynamic tag name via createElement equivalent — just use intrinsic elements
            const rtag = tagName === "i" ? <i>{children}</i> : <b>{children}</b>;
            lastPart.push(rtag);
            partsStack.push(children);
            continue;
        }
    }
    partsStack[partsStack.length - 1].push(titleString.substring(lastIdx));
    return partsStack[0];
}

export function getBlockHeaderIcon(blockIcon: string, blockData: Block): JSX.Element {
    if (util.isBlank(blockIcon)) {
        blockIcon = "square";
    }
    let iconColor = blockData?.meta?.["icon:color"];
    if (iconColor && !iconColor.match(colorRegex)) {
        iconColor = null;
    }
    let iconStyle: JSX.CSSProperties = null;
    if (!util.isBlank(iconColor)) {
        iconStyle = { color: iconColor };
    }
    const iconClass = util.makeIconClass(blockIcon, true);
    if (iconClass != null) {
        return <i style={iconStyle} class={clsx(`block-frame-icon`, iconClass)} />;
    }
    return null;
}

interface ConnectionButtonProps {
    connection: string;
    changeConnModalAtom: { (): boolean; _set(v: boolean | ((prev: boolean) => boolean)): void };
    ref?: { current: HTMLDivElement | null };
}

export function computeConnColorNum(connStatus: ConnStatus): number {
    // activeconnnum is 1-indexed, so we need to adjust for when mod is 0
    const connColorNum = (connStatus?.activeconnnum ?? 1) % NumActiveConnColors;
    if (connColorNum == 0) {
        return NumActiveConnColors;
    }
    return connColorNum;
}

export function ConnectionButton({ connection, changeConnModalAtom, ref }: ConnectionButtonProps): JSX.Element {
    const [connModalOpen, setConnModalOpen] = createSignal(changeConnModalAtom());
    const isLocal = util.isBlank(connection);
    const connStatusAtom = getConnStatusAtom(connection);
    const connStatus = createMemo(() => connStatusAtom());
    let showDisconnectedSlash = false;
    const connColorNum = createMemo(() => computeConnColorNum(connStatus()));
    const color = createMemo(() => `var(--conn-icon-color-${connColorNum()})`);
    const clickHandler = function () {
        changeConnModalAtom._set(true);
        setConnModalOpen(true);
    };
    let titleText = null;
    let shouldSpin = false;

    const getConnIcon = (): JSX.Element => {
        const cs = connStatus();
        if (isLocal) {
            return (
                <i
                    class={clsx(util.makeIconClass("laptop", false), "fa-stack-1x")}
                    style={{ color: "var(--grey-text-color)", "margin-right": "2px" }}
                />
            );
        }
        if (cs?.status == "connecting") {
            return (
                <div class="connecting-svg">
                    <img src={dotsUrl} />
                </div>
            );
        }
        const iconName = "arrow-right-arrow-left";
        return (
            <i
                class={clsx(util.makeIconClass(iconName, false), "fa-stack-1x")}
                style={{ color: color(), "margin-right": "2px" }}
            />
        );
    };

    const getTitleText = (): string => {
        const cs = connStatus();
        if (isLocal) return "Connected to Local Machine";
        if (cs?.status == "connecting") return "Connecting to " + connection;
        if (cs?.status == "error") {
            let t = "Error connecting to " + connection;
            if (cs?.error != null) t += " (" + cs.error + ")";
            return t;
        }
        if (!cs?.connected) return "Disconnected from " + connection;
        return "Connected to " + connection;
    };

    const getShowDisconnectedSlash = (): boolean => {
        const cs = connStatus();
        if (isLocal) return false;
        return cs?.status == "error" || !cs?.connected;
    };

    return (
        <div ref={(el) => { if (ref) ref.current = el; }} class={clsx("connection-button")} onClick={clickHandler} title={getTitleText()}>
            <span class={clsx("fa-stack connection-icon-box", shouldSpin ? "fa-spin" : null)}>
                {getConnIcon()}
                <i
                    class="fa-slash fa-solid fa-stack-1x"
                    style={{
                        color: color(),
                        "margin-right": "2px",
                        "text-shadow": "0 1px black, 0 1.5px black",
                        opacity: getShowDisconnectedSlash() ? 1 : 0,
                    }}
                />
            </span>
            {isLocal ? null : <div class="connection-name ellipsis">{connection}</div>}
        </div>
    );
}

export function Input({ decl, className, preview }: { decl: HeaderInput; className: string; preview: boolean }): JSX.Element {
    const { value, ref, isDisabled, onChange, onKeyDown, onFocus, onBlur } = decl;
    return (
        <div class="input-wrapper">
            <input
                ref={!preview && ref ? (el) => { ref.current = el; } : undefined}
                disabled={isDisabled}
                class={className}
                value={value}
                onChange={(e) => onChange?.(e as any)}
                onKeyDown={(e) => onKeyDown?.(e as any)}
                onFocus={(e) => onFocus?.(e as any)}
                onBlur={(e) => onBlur?.(e as any)}
                onDragStart={(e) => e.preventDefault()}
            />
        </div>
    );
}

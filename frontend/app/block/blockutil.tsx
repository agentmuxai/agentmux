// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { NumActiveConnColors } from "@/app/block/blockframe";
import { getConnStatusAtom } from "@/app/store/global";
import * as util from "@/util/util";
import clsx from "clsx";
import type { JSX } from "solid-js";
import { createEffect, createMemo, createSignal, Show } from "solid-js";
import dotsUrl from "../asset/dots-anim-4.svg?url";

const colorRegex = /^((#[0-9a-f]{6,8})|([a-z]+))$/;

export function blockViewToIcon(view: string): string {
    if (view == "term") {
        return "terminal";
    }
    if (view == "help") {
        return "circle-question";
    }
    if (view == "swarm") {
        return "diagram-project";
    }
    if (view == "drone") {
        return "diagram-project";
    }
    if (view == "media") {
        return "photo-film";
    }
    return "square";
}

const VIEW_LABELS: Record<string, string> = {
    agent: "Agent",
    browser: "Browser",
    drone: "Drone",
    editor: "Editor",
    help: "Help",
    identity: "Identity",
    media: "Media",
    memory: "Memory",
    swarm: "Swarm",
    sysinfo: "Sysinfo",
    term: "Terminal",
    warden: "Warden",
};

export function blockViewToName(view: string): string {
    if (util.isBlank(view)) {
        return "(No View)";
    }
    return VIEW_LABELS[view] ?? view;
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

export function ConnectionButton(props: ConnectionButtonProps): JSX.Element {
    // Reads `props.connection` reactively throughout (never destructured) —
    // codex P2 on PR #3134: destructuring it once, like the previous
    // version of this component did, freezes `isLocal`/`connStatusAtom` at
    // whatever `connection` was on first mount. That was harmless as long
    // as every blockId change forced a remount (the header's own remount
    // boundary), but a hoisted, switch-surviving header
    // (SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md) can now change
    // which block's `connection` this same mounted instance should reflect
    // without remounting at all.
    const [connModalOpen, setConnModalOpen] = createSignal(props.changeConnModalAtom());
    const isLocal = createMemo(() => util.isBlank(props.connection));
    const connStatus = createMemo(() => getConnStatusAtom(props.connection)());
    const connColorNum = createMemo(() => computeConnColorNum(connStatus()));
    const color = createMemo(() => `var(--conn-icon-color-${connColorNum()})`);
    const clickHandler = function () {
        props.changeConnModalAtom._set(true);
        setConnModalOpen(true);
    };
    let shouldSpin = false;

    const getConnIcon = (): JSX.Element => {
        const cs = connStatus();
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
        const connection = props.connection;
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
        return cs?.status == "error" || !cs?.connected;
    };

    // Codex P1 on PR #3157: a plain callback ref fires ONCE, when the
    // element is created. That was fine while `props.ref` was a stable
    // per-mount object, but a hoisted pane header
    // (SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md) stays mounted
    // across a tab switch and re-points `props.ref` at the newly-active
    // block's holder — this `<Show>` stays true the whole time when both
    // tabs have connections, so the element is never recreated and the new
    // holder would keep `current: null`. TypeAheadModal then calls
    // `anchorRef.current.getBoundingClientRect()` unconditionally and the
    // connection selector THROWS instead of opening. An effect re-runs
    // whenever `props.ref` changes identity, so each holder that becomes
    // current gets populated with the live element.
    const [btnEl, setBtnEl] = createSignal<HTMLDivElement | null>(null);
    createEffect(() => {
        const holder = props.ref;
        const el = btnEl();
        if (holder) holder.current = el;
    });

    return (
        <Show when={!isLocal()}>
            <div
                ref={(el) => setBtnEl(el)}
                class={clsx("connection-button")}
                onClick={clickHandler}
                title={getTitleText()}
            >
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
                <div class="connection-name ellipsis">{props.connection}</div>
            </div>
        </Show>
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

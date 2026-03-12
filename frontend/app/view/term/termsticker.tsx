// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { createBlock } from "@/store/global";
import { getWebServerEndpoint } from "@/util/endpoints";
import { stringToBase64 } from "@/util/util";
import clsx from "clsx";
import { For, type JSX } from "solid-js";
type CSSProperties = JSX.CSSProperties;
import "./term.scss";

type StickerType = {
    position: "absolute";
    top?: number;
    left?: number;
    right?: number;
    bottom?: number;
    width?: number;
    height?: number;
    color?: string;
    opacity?: number;
    pointerevents?: boolean;
    fontsize?: number;
    transform?: string;

    stickertype: "icon" | "image" | "gauge";
    icon?: string;
    imgsrc?: string;
    clickcmd?: string;
    clickblockdef?: BlockDef;
};

type StickerTermConfig = {
    charWidth: number;
    charHeight: number;
    rows: number;
    cols: number;
    blockId: string;
};

function convertWidthDimToPx(dim: number, config: StickerTermConfig): number | undefined {
    if (dim == null) return undefined;
    return dim * config.charWidth;
}

function convertHeightDimToPx(dim: number, config: StickerTermConfig): number | undefined {
    if (dim == null) return undefined;
    return dim * config.charHeight;
}

function TermSticker(props: { sticker: StickerType; config: StickerTermConfig }): JSX.Element {
    const { sticker, config } = props;
    const style: Record<string, any> = {
        position: sticker.position,
        top: convertHeightDimToPx(sticker.top, config),
        left: convertWidthDimToPx(sticker.left, config),
        right: convertWidthDimToPx(sticker.right, config),
        bottom: convertHeightDimToPx(sticker.bottom, config),
        width: convertWidthDimToPx(sticker.width, config),
        height: convertHeightDimToPx(sticker.height, config),
        color: sticker.color,
        fontSize: sticker.fontsize,
        transform: sticker.transform,
        opacity: sticker.opacity,
        fill: sticker.color,
        stroke: sticker.color,
    };
    if (sticker.pointerevents) {
        style.pointerEvents = "auto";
    }
    if (style.width != null) {
        style.overflowX = "hidden";
    }
    if (style.height != null) {
        style.overflowY = "hidden";
    }
    let clickHandler: (() => void) | null = null;
    if (sticker.pointerevents && (sticker.clickcmd || sticker.clickblockdef)) {
        style.cursor = "pointer";
        clickHandler = () => {
            console.log("clickHandler", sticker.clickcmd, sticker.clickblockdef);
            if (sticker.clickcmd) {
                const b64data = stringToBase64(sticker.clickcmd);
                RpcApi.ControllerInputCommand(TabRpcClient, { blockid: config.blockId, inputdata64: b64data });
            }
            if (sticker.clickblockdef) {
                createBlock(sticker.clickblockdef);
            }
        };
    }
    if (sticker.stickertype == "icon") {
        return (
            <div class="term-sticker" style={style as any} onClick={clickHandler}>
                <i class={clsx("fa", "fa-" + sticker.icon)} />
            </div>
        );
    }
    if (sticker.stickertype == "image") {
        if (sticker.imgsrc == null) return null;
        const streamingUrl =
            getWebServerEndpoint() + "/wave/stream-local-file?path=" + encodeURIComponent(sticker.imgsrc);
        return (
            <div class="term-sticker term-sticker-image" style={style as any} onClick={clickHandler}>
                <img src={streamingUrl} />
            </div>
        );
    }
    return null;
}

export function TermStickers(props: { config: StickerTermConfig }): JSX.Element {
    const stickers: StickerType[] = [];
    if (props.config.blockId.startsWith("d1eaddcb")) {
        stickers.push({
            position: "absolute",
            top: 5,
            right: 7,
            stickertype: "icon",
            icon: "paw",
            color: "#40cc40aa",
            fontsize: 30,
            transform: "rotate(-18deg)",
            pointerevents: true,
            clickcmd: "ls\n",
        });
        stickers.push({
            position: "absolute",
            top: 8,
            right: 8,
            stickertype: "icon",
            icon: "paw",
            color: "#4040ccaa",
            fontsize: 30,
            transform: "rotate(-20deg)",
            pointerevents: true,
            clickcmd: "git status\n",
        });
        stickers.push({
            position: "absolute",
            top: 2,
            right: 25,
            width: 20,
            stickertype: "gauge",
            opacity: 0.7,
        });
    }
    return (
        <div class="term-stickers">
            <For each={stickers}>
                {(sticker) => <TermSticker sticker={sticker} config={props.config} />}
            </For>
        </div>
    );
}

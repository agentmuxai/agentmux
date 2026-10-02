// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Opening a file from the Files pane by its kind (§5.4): text and code in an
 * Editor pane, images, video and audio in a Media pane, both beside the Files
 * pane; anything else with the OS's default app. A program asks first.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.4.
 */

import { AUDIO_EXTENSIONS, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS } from "@/app/element/local-media";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { extensionOf } from "./files-sort";

export type OpenTarget = "editor" | "media" | "os" | "program";

/** Formats the Editor can't show and that have a better home elsewhere. */
const BINARY = new Set(
    (
        "pdf doc docx xls xlsx ppt pptx odt ods odp rtf pages numbers key " +
        "zip 7z rar gz tgz bz2 xz zst tar iso dmg cab jar war apk " +
        "dll so dylib lib a o obj class pyc wasm bin dat db sqlite sqlite3 mdb " +
        "psd ai sketch fig xcf blend fbx glb gltf stl " +
        "mp3 flac ogg m4a aac wma mkv avi wmv flv mpg mpeg m4v 3gp " +
        "bmp tif tiff heic heif raw cr2 nef ico icns " +
        "ttf otf woff woff2 eot"
    ).split(" ")
);

/** Files that run something when the OS opens them. */
const PROGRAM = new Set("exe msi com scr lnk app pkg appx msix url reg".split(" "));

export function openTargetOf(name: string): OpenTarget {
    const ext = extensionOf(name);
    if (PROGRAM.has(ext)) return "program";
    if (IMAGE_EXTENSIONS.includes(ext) || VIDEO_EXTENSIONS.includes(ext) || AUDIO_EXTENSIONS.includes(ext)) return "media";
    if (BINARY.has(ext)) return "os";
    return "editor";
}

/** Opens `path` in a new pane of `view` to the right of the Files pane. */
export async function openInPane(view: "editor" | "media", path: string, besideBlockId: string): Promise<void> {
    await TabRpcClient.rpcCall(
        "pane.open",
        { view, file: path, split_direction: "right", split_reference_block_id: besideBlockId },
        {}
    );
}

export async function openWithOs(path: string): Promise<void> {
    await RpcApi.FsOpenCommand(TabRpcClient, { path });
}

export async function revealInOs(path: string): Promise<void> {
    await RpcApi.FsRevealCommand(TabRpcClient, { path });
}

/** Opens a terminal in `dir`, to the right of the Files pane. */
export async function openTerminalHere(dir: string, besideBlockId: string): Promise<void> {
    await TabRpcClient.rpcCall(
        "pane.open",
        { view: "term", cwd: dir, split_direction: "right", split_reference_block_id: besideBlockId },
        {}
    );
}

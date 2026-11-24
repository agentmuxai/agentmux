// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What srv said about each file a message names (spec §5): asked lazily, once per path and pane for the session, so a
 * long transcript never makes a request per path. A link reads its answer here; until it has one it is drawn as
 * unchecked, and a dead one turns muted with the reason. Nothing here reads a file or builds a path.
 */

import { createStore } from "solid-js/store";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { siteOf } from "./entities/bare-domain";

export type FileCheck = { state: "found" | "missing" | "outside" | "limited"; reason: string; resolved: string };

const [answers, setAnswers] = createStore<Record<string, FileCheck>>({});
const asking = new Map<string, Promise<FileCheck>>();

const keyOf = (blockId: string, path: string) => `${blockId}\n${path}`;

const UNAVAILABLE: FileCheck = { state: "limited", reason: "The file couldn't be checked right now.", resolved: "" };

/** The answer srv gave, if it has; reactive. */
export function fileCheck(blockId: string, path: string): FileCheck | undefined {
    return answers[keyOf(blockId, path)];
}

async function ask(blockId: string, path: string): Promise<FileCheck> {
    try {
        const got = await RpcApi.FsLinkFileCheckCommand(TabRpcClient, { path, block_id: blockId }, {});
        const state = got?.state === "found" || got?.state === "missing" || got?.state === "outside" || got?.state === "limited" ? got.state : "limited";
        return { state, reason: got?.reason ?? "", resolved: got?.resolved ?? "" };
    } catch {
        return UNAVAILABLE;
    }
}

/** Ask once (a hover or a focus): a later call, or a second link to the same path, reuses the answer. An answer of
 *  "limited" is not kept, so it can be asked again later. */
export function requestFileCheck(blockId: string, path: string): void {
    if (fileCheck(blockId, path)) return;
    void checkFileNow(blockId, path);
}

/** Ask now and wait for the answer (a click): always fresh, because the click is what opens the file. */
export async function checkFileNow(blockId: string, path: string): Promise<FileCheck> {
    const key = keyOf(blockId, path);
    const pending = asking.get(key);
    if (pending) return pending;
    const run = ask(blockId, path).then((got) => {
        asking.delete(key);
        if (got.state === "limited") setAnswers(key, undefined as unknown as FileCheck);
        else setAnswers(key, got);
        return got;
    });
    asking.set(key, run);
    return run;
}

/** Whether a link to this file should be drawn dead, and why. */
export function deadReason(blockId: string, path: string): string | undefined {
    // A name that is also a web site is not dead when no such file is there: it is the site.
    if (siteOf(path)) return undefined;
    const got = fileCheck(blockId, path);
    return got && (got.state === "missing" || got.state === "outside") ? got.reason : undefined;
}

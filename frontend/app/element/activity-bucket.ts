// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The kinds of long-running work and the 3-cell ASCII animation each one gets
// (SPEC_LONG_RUNNING_ACTIVITY_ANIMATIONS_2026_11_02.md §3-§5). The rotating slash
// (AsciiSpinner) stays the one sign that an agent's turn is in progress; none of
// these moves with `|`, `/`, `-` or `\`.
//
// Every animation is exactly 3 cells, so a row never shifts as frames change, and runs
// off the shared 120 ms ticker (shared-ticker.ts): a bucket's frame period is a multiple
// of it, and its frame is `floor(tick / (period / 120)) % frames.length`.

import { commandBucketId } from "./command-bucket";
import { TICKER_MS } from "./shared-ticker";

export type BucketId =
    | "build"
    | "test"
    | "send"
    | "receive"
    | "install"
    | "search"
    | "deploy"
    | "delegate"
    | "compact"
    | "wait"
    | "hold"
    | "other";

export interface Bucket {
    id: BucketId;
    /** The tooltip's word and the Working row's: "Building". */
    label: string;
    /** Each frame is exactly 3 cells; a space is a blank cell. */
    frames: readonly string[];
    /** A multiple of the ticker's 120 ms. */
    periodMs: number;
    /** What shows when the motion policy reduces `status`: the kind of work stays readable. */
    still: string;
}

export const BUCKETS: Record<BucketId, Bucket> = {
    build: { id: "build", label: "Building", frames: ["+x+", "x+x"], periodMs: 240, still: "+x+" },
    test: { id: "test", label: "Testing", frames: ["v  ", "vv ", "vvv", "   "], periodMs: 240, still: "vvv" },
    send: { id: "send", label: "Sending", frames: [">  ", " > ", "  >"], periodMs: 120, still: " > " },
    receive: { id: "receive", label: "Receiving", frames: ["  <", " < ", "<  "], periodMs: 120, still: " < " },
    install: { id: "install", label: "Installing", frames: ["[ ]", "[.]", "[o]", "[#]"], periodMs: 240, still: "[#]" },
    search: { id: "search", label: "Searching", frames: ["o  ", " o ", "  o", " o "], periodMs: 120, still: " o " },
    deploy: { id: "deploy", label: "Deploying", frames: [" . ", " : ", " i ", " ! ", " ^ ", "   "], periodMs: 240, still: " ^ " },
    delegate: { id: "delegate", label: "Delegated", frames: ["-o-", ">o<", "-o-", "<o>"], periodMs: 240, still: "-o-" },
    compact: { id: "compact", label: "Compacting", frames: ["< >", "<.>", "<:>", "<#>"], periodMs: 240, still: "<#>" },
    wait: { id: "wait", label: "Waiting", frames: ["( )", "(.)", "(o)", "(O)", "(o)", "(.)"], periodMs: 360, still: "(o)" },
    hold: { id: "hold", label: "Held", frames: ["z  ", "zZ ", "zZz", "   "], periodMs: 480, still: "zZz" },
    other: { id: "other", label: "Running", frames: [".  ", ".. ", "...", "   "], periodMs: 360, still: "..." },
};

/** The frame of `bucket` at shared tick `tick`. */
export function bucketFrame(bucket: Bucket, tick: number): string {
    const step = Math.floor(tick / (bucket.periodMs / TICKER_MS));
    return bucket.frames[step % bucket.frames.length];
}

/** A running tool call shows its bucket animation once it has run this long, so quick calls don't flicker. */
export const BUCKET_AFTER_MS = 3000;

/** The tooltip: the bucket's name and how long it has run ("Building, 4m 12s"). */
export function bucketTooltip(bucket: Bucket, elapsed: string): string {
    return `${bucket.label}, ${elapsed}`;
}

/** What an event already knows about its own kind (the first rule). */
export type BucketSource =
    | "compaction"
    | "subagent"
    | "quota-hold"
    | "schedule"
    | "hibernation"
    | "approval"
    | "review-wait"
    | "navigation"
    | "container";

const SOURCE_BUCKETS: Record<BucketSource, BucketId> = {
    compaction: "compact",
    subagent: "delegate",
    "quota-hold": "hold",
    schedule: "hold",
    hibernation: "hold",
    approval: "wait",
    "review-wait": "wait",
    navigation: "receive",
    container: "install",
};

/** Tools that name their kind of work (the second rule), by lowercase name. */
const TOOL_BUCKETS: Record<string, BucketId> = {
    grep: "search",
    glob: "search",
    search_history: "search",
    websearch: "search",
    webfetch: "receive",
    agent: "delegate",
    task: "delegate",
};

export interface BucketInput {
    /** What the event itself says it is. */
    source?: BucketSource;
    /** The tool's name (`Grep`, `Agent`), when it is a tool call. */
    tool?: string;
    /** The shell command, for a Bash call or a shell. */
    command?: string;
}

/**
 * The bucket a running operation belongs in, first match wins: what its source says, then its tool's
 * name, then its shell command by its first real program and subcommand, otherwise the fallback.
 * Called once per operation; it never changes while the same operation runs.
 */
export function activityBucket(input: BucketInput): Bucket {
    if (input.source) return BUCKETS[SOURCE_BUCKETS[input.source]];
    const byTool = input.tool ? TOOL_BUCKETS[input.tool.toLowerCase()] : undefined;
    if (byTool) return BUCKETS[byTool];
    const byCommand = input.command ? commandBucketId(input.command) : undefined;
    return BUCKETS[byCommand ?? "other"];
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Is every agent's transcript intact after what the run did to it? (`--verify-transcripts`.) Each agent's transcript is
// read through `agent:open_tail` (the output stream's line count, its newest line, which stream it is, and a hash of the lines)
// once the turns are over, and again after everything else the run did (closing and reopening the panes). A transcript only
// ever grows: the second reading must start with exactly the first reading's lines. It may be longer, and with the lab's fake
// CLI it is, by one scripted turn per reopen: the reopen starts the CLI again, srv sends its startup message, and the fake CLI,
// which remembers nothing, plays the next turn it has. So what is checked is that nothing was lost or changed, that the stream
// is the same, that the newest line is the end of a turn, and that the tool calls in the transcript are at least the tool calls
// the scripted turns made. A soak that reopens every agent at the end is the case this is for.

/** The number of tool calls the played turns of the script made (the `tool` steps), for the first `turns` regular turns and, optionally, the heavy ones. */
export function expectedToolUses(script, regularTurns, heavyTurns = 0) {
    const toolSteps = (turn) => (turn?.steps ?? []).filter((s) => s.tool).length;
    let n = 0;
    const regular = script.turns.filter((t) => !t.match);
    for (let i = 0; i < Math.min(regularTurns, regular.length); i++) n += toolSteps(regular[i]);
    for (let k = 1; k <= heavyTurns; k++) n += toolSteps(script.turns.find((t) => t.match === `[heavy ${k}]`));
    return n;
}

/** Above this many bytes the lab does not read a whole transcript just to count its tool calls (the line count and newest line are still checked). */
export const COUNT_LIMIT_BYTES = 16 * 1024 * 1024;

/** Runs in the page: one agent's transcript, as { totalLines, stream, gen, size, last, toolUses, hashAll, hashFirst }. Everything but the first five is null above COUNT_LIMIT_BYTES. `hashFirst`: of the first `prefixLines` lines. */
export function readTranscriptScript(blockId, prefixLines = null) {
    return `(async () => {
  try {
    const call = (tail) => RpcApi.AgentOpenTailCommand(TabRpcClient, { block_id: ${JSON.stringify(blockId)}, filename: "output", tail_bytes: tail });
    const head = await call(4096);
    const out = { totalLines: head.total_lines ?? null, stream: head.stream ?? null, gen: head.gen ?? null, size: head.size ?? null, last: head.lines.length ? head.lines[head.lines.length - 1] : null, toolUses: null, hashAll: null, hashFirst: null };
    if (head.size != null && head.size <= ${COUNT_LIMIT_BYTES}) {
      const all = await call(head.size + 1);
      if (all.has_more_before === false && all.total_lines === head.total_lines) {
        let n = 0;
        const fnv = (upTo) => { let h = 0x811c9dc5; for (let i = 0; i < upTo && i < all.lines.length; i++) { const line = all.lines[i]; for (let k = 0; k < line.length; k++) { h ^= line.charCodeAt(k); h = Math.imul(h, 0x01000193) >>> 0; } h ^= 10; h = Math.imul(h, 0x01000193) >>> 0; } return h; };
        for (const line of all.lines) { let at = 0; while ((at = line.indexOf('"type":"tool_use"', at)) >= 0) { n++; at += 17; } }
        out.toolUses = n;
        out.hashAll = fnv(all.lines.length);
        out.hashFirst = ${prefixLines == null ? "null" : "fnv(" + Number(prefixLines) + ")"};
      }
    }
    return out;
  } catch (e) { return { error: String(e && e.message ? e.message : e) }; }
})()`;
}

/** Which stream a transcript is: "g" (the agent's global store), "b" (the block's), or whatever the prefix is. */
export function streamKind(stream) {
    const m = /^([a-z]+):/.exec(stream ?? "");
    return m ? m[1] : null;
}

const isResultLine = (line) => {
    try {
        return JSON.parse(line)?.type === "result";
    } catch {
        return false;
    }
};

/**
 * Compares the two readings of every agent. `before`/`after`: [{ agent, read }] (read as `readTranscriptScript` returns it, the
 * second with `prefixLines` = the first's line count); `expected`: { agent: tool calls the script made for it }. Returns { ok, problems, agents }.
 */
export function checkTranscripts(before, after, expected = {}) {
    const problems = [];
    const agents = before.map((b) => {
        const a = after.find((x) => x.agent === b.agent);
        const row = {
            agent: b.agent,
            stream: b.read?.stream ?? null,
            streamKind: streamKind(b.read?.stream),
            linesBefore: b.read?.totalLines ?? null,
            linesAfter: a?.read?.totalLines ?? null,
            linesAdded: a?.read && b.read ? (a.read.totalLines ?? 0) - (b.read.totalLines ?? 0) : null,
            // The second reading begins with exactly the first one's lines; null when a transcript was too big to read whole.
            startsWithBefore: b.read?.hashAll != null && a?.read?.hashFirst != null ? b.read.hashAll === a.read.hashFirst : null,
            lastIsResult: a?.read?.last != null && isResultLine(a.read.last),
            toolUses: a?.read?.toolUses ?? null,
            toolUsesExpected: expected[b.agent] ?? null,
        };
        const bad = (why) => problems.push(`${b.agent}: ${why}`);
        if (b.read?.error || a?.read?.error) bad(`could not read the transcript (${b.read?.error ?? a?.read?.error})`);
        else if (!a) bad("not read the second time");
        else {
            if (row.linesAfter < row.linesBefore) bad(`${row.linesBefore} lines before, ${row.linesAfter} after: lines were lost`);
            if (row.startsWithBefore === false) bad("the lines it had before were changed");
            if (row.linesAdded === 0 && b.read.last !== a.read.last) bad("the newest line changed with no line added");
            if (b.read.stream !== a.read.stream) bad(`the stream changed (${b.read.stream} to ${a.read.stream})`);
            if (!row.lastIsResult) bad("the newest line is not the end of a turn");
            if (row.toolUses != null && row.toolUsesExpected != null && row.toolUses < row.toolUsesExpected) bad(`${row.toolUses} tool calls in the transcript, the script made ${row.toolUsesExpected}`);
        }
        return row;
    });
    return { ok: problems.length === 0, problems, agents };
}

/** One line for the run's summary. */
export function transcriptsLine(t) {
    if (!t) return "";
    const kinds = [...new Set(t.agents.map((a) => a.streamKind))].join("/");
    const added = t.agents.filter((a) => a.linesAdded > 0).length;
    const unchecked = t.agents.filter((a) => a.startsWithBefore == null).length;
    return t.ok
        ? `Transcripts: ${t.agents.length} agents' transcripts (stream ${kinds}) kept every line they had before the reopens (${added} grew, by the turn the restarted CLI played), end on a finished turn, and hold at least the scripted tool calls${unchecked ? `; ${unchecked} too big to compare line by line` : ""}.`
        : `TRANSCRIPT PROBLEM (stream ${kinds}): ${t.problems.join("; ")}`;
}

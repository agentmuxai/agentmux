// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Run: node --test tools/tests/lib/instance-discovery.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { listLiveInstances, resolveInstance } from "./instance-discovery.mjs";

function fakeRoot(instances) {
    const root = mkdtempSync(join(tmpdir(), "am-discovery-"));
    for (const [i, inst] of instances.entries()) {
        const dir = join(root, "channels", inst.channel, "versions", inst.version, "data");
        mkdirSync(dir, { recursive: true });
        writeFileSync(join(dir, "authkey.dev"), JSON.stringify({
            version: 1,
            auth_key: `key-${i}`,
            web_endpoint: inst.web || `127.0.0.1:900${i}`,
            ws_endpoint: `127.0.0.1:910${i}`,
            host_pid: inst.pid,
            instance: `v${inst.version}`,
            data_dir: dir,
            ...(inst.debugPort !== undefined ? { debug_port: inst.debugPort } : {}),
        }));
    }
    return root;
}

const aliveOnly = (pids) => (pid) => pids.includes(pid);

test("a dead instance's authkey is ignored", () => {
    const root = fakeRoot([
        { channel: "alpha", version: "0.56.4", pid: 111, debugPort: 42149 },
        { channel: "stale", version: "0.56.3", pid: 222, debugPort: 9222 },
    ]);
    const live = listLiveInstances({ root, alive: aliveOnly([111]) });
    assert.equal(live.length, 1);
    assert.equal(live[0].hostPid, 111);
});

test("the debug port comes from the file, not a constant", () => {
    const root = fakeRoot([{ channel: "alpha", version: "0.56.4", pid: 111, debugPort: 42149 }]);
    const [i] = listLiveInstances({ root, alive: aliveOnly([111]) });
    // 42149, not 9222: the second instance to start never gets the preferred
    // port, which is the whole reason this module exists.
    assert.equal(i.debugPort, 42149);
});

test("an instance predating the field reports null rather than defaulting", () => {
    const root = fakeRoot([{ channel: "alpha", version: "0.56.2", pid: 111 }]);
    const [i] = listLiveInstances({ root, alive: aliveOnly([111]) });
    // Silently substituting 9222 here is precisely the bug: it would target
    // whichever instance happens to hold that port.
    assert.equal(i.debugPort, null);
});

test("refuses to guess when several instances are live", () => {
    const root = fakeRoot([
        { channel: "alpha", version: "0.56.4", pid: 111, debugPort: 42149 },
        { channel: "beta", version: "0.56.3", pid: 222, debugPort: 9222 },
    ]);
    assert.throws(
        () => resolveInstance({ root, alive: aliveOnly([111, 222]) }),
        /refusing to guess/,
        "ambiguity must be an error, never a silent pick"
    );
});

test("a match selects one instance", () => {
    const root = fakeRoot([
        { channel: "alpha", version: "0.56.4", pid: 111, debugPort: 42149 },
        { channel: "beta", version: "0.56.3", pid: 222, debugPort: 9222 },
    ]);
    const got = resolveInstance({ match: "beta", root, alive: aliveOnly([111, 222]) });
    assert.equal(got.hostPid, 222);
});

test("a match that hits nothing errors and lists what is live", () => {
    const root = fakeRoot([{ channel: "alpha", version: "0.56.4", pid: 111, debugPort: 42149 }]);
    assert.throws(
        () => resolveInstance({ match: "nope", root, alive: aliveOnly([111]) }),
        /no live instance matching/
    );
});

test("no live instances is an error, not an empty pick", () => {
    const root = fakeRoot([{ channel: "alpha", version: "0.56.4", pid: 111, debugPort: 42149 }]);
    assert.throws(() => resolveInstance({ root, alive: () => false }), /no live AgentMux instance/);
});

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// scripts/mem-sampler.sh prints a memory line into a release build's log. It must read only numbers and an
// executable's name, never fail or hang the build, and end by itself. The macOS tools are stubs here.
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, describe, expect, it } from "vitest";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SAMPLER = join(ROOT, "scripts/mem-sampler.sh").replace(/\\/g, "/");
const tmp = mkdtempSync(join(tmpdir(), "mem-sampler-"));
afterAll(() => rmSync(tmp, { recursive: true, force: true }));

const fwd = (p) => p.replace(/\\/g, "/");
function stubs(name, files) {
    const dir = join(tmp, name);
    mkdirSync(dir, { recursive: true });
    for (const [file, body] of Object.entries(files)) writeFileSync(join(dir, file), `#!/bin/bash\n${body}\n`, { mode: 0o755 });
    return fwd(dir);
}
const bash = (script, dir, env = {}) =>
    spawnSync("bash", ["-c", script], {
        encoding: "utf8",
        env: { ...process.env, ...env, PATH: `${dir}${delimiter}${process.env.PATH}` },
        timeout: 60000,
    });

const VM_STAT = `Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                               65536.
Pages active:                            262144.
Pages inactive:                          100000.
Pages speculative:                         4096.
Pages wired down:                        131072.
Pages occupied by compressor:             32768.
`;
const macTools = stubs("mac", {
    vm_stat: `cat <<'EOF'\n${VM_STAT}EOF`,
    sysctl: `echo "total = 2048.00M  used = 1180.25M  free = 867.75M  (encrypted)"`,
    memory_pressure: `echo "System-wide memory free percentage: 41%"`,
    ps: `printf '%s\\n' "  2154496 /Users/runner/.rustup/toolchains/x/bin/rustc" "   100 /bin/sh"`,
});

describe("mem_line", { timeout: 60000 }, () => {
    it("prints free, active, wired and compressed memory, swap in use and the biggest process by name", () => {
        const r = bash(`. ${SAMPLER}; mem_line`, macTools);
        expect(r.status).toBe(0);
        expect(r.stdout.trim()).toMatch(
            /^mem: t=\d+s free=1\.1G\(41%\) active=4\.0G wired=2\.0G compressed=0\.5G swap_used=1180\.25M top=rustc 2104MB$/,
        );
    });

    it("prints nothing from the environment or from a process's arguments", () => {
        const withArgs = stubs("mac-args", {
            vm_stat: `cat <<'EOF'\n${VM_STAT}EOF`,
            sysctl: `echo "total = 0.00M  used = 0.00M  free = 0.00M"`,
            memory_pressure: `echo "System-wide memory free percentage: 50%"`,
            // a process whose command line carries a secret: ps is asked for the executable name only
            ps: `case "$*" in *args*|*command*) echo "leaked s3cr3t-token";; *) echo "  2048 /usr/bin/tool";; esac`,
        });
        const r = bash(`. ${SAMPLER}; mem_line`, withArgs, { AGENTMUX_TEST_SECRET: "s3cr3t-token", GH_TOKEN: "s3cr3t-token" });
        expect(r.status).toBe(0);
        expect(r.stdout).not.toContain("s3cr3t-token");
        expect(r.stdout).toContain("top=tool 2MB");
    });

    it("prints a question mark for what it could not read, and still succeeds", () => {
        const failing = stubs("mac-fail", {
            vm_stat: "exit 1",
            sysctl: "exit 1",
            memory_pressure: "exit 1",
            ps: "exit 1",
        });
        const r = bash(`. ${SAMPLER}; mem_line; echo "rc=$?"`, failing);
        expect(r.stdout).toMatch(/free=\?\(\?%\) active=\? wired=\? compressed=\? swap_used=\? top=\?/);
        expect(r.stdout).toContain("rc=0");
    });
});

describe("the sampler's life", { timeout: 60000 }, () => {
    it("stops when asked, and stopping when nothing was started is fine", () => {
        const r = bash(
            `set -euo pipefail; . ${SAMPLER}; mem_sampler_stop; MEM_SAMPLER_INTERVAL=1 mem_sampler_start; sleep 2.5; mem_sampler_stop; echo "stopped rc=$?"`,
            macTools,
        );
        expect(r.status).toBe(0);
        expect(r.stdout).toContain("stopped rc=0");
        const lines = r.stdout.split("\n").filter((l) => l.startsWith("mem: "));
        expect(lines.length).toBeGreaterThanOrEqual(1);
        expect(lines.length).toBeLessThanOrEqual(4);
    });

    it("ends by itself when the script that started it is gone", () => {
        // a pid that has just ended: the sampler's first look at its parent fails and it prints nothing
        const r = bash(`sleep 0 & dead=$!; wait $dead; bash ${SAMPLER} 1 $dead 5; echo "rc=$?"`, macTools);
        expect(r.stdout.trim()).toBe("rc=0");
    });

    it("never prints more lines than its limit", () => {
        const r = bash(`bash ${SAMPLER} 0 $$ 3`, macTools);
        expect(r.status).toBe(0);
        expect(r.stdout.split("\n").filter((l) => l.startsWith("mem: "))).toHaveLength(3);
    });
});

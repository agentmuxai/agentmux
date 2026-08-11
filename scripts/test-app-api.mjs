// Ad-hoc App API smoke test — exercises identity.*, bundle.*, memory.* as AgentX.
// Run: node scripts/test-app-api.mjs
import WebSocket from "ws";

const PORT = process.env.AGENTMUX_LOCAL_URL?.split(":").pop() ?? "61018";
const KEY = process.env.AGENTMUX_AUTH_KEY;
const AGENT = process.env.AGENTMUX_AGENT_ID ?? "AgentX";
const url = `ws://127.0.0.1:${PORT}/ws?authkey=${KEY}`;

const ws = new WebSocket(url);
const pending = new Map();
let n = 0;

function call(command, data) {
    const reqid = `t${++n}`;
    return new Promise((resolve, reject) => {
        pending.set(reqid, { resolve, reject });
        ws.send(JSON.stringify({ wscommand: "rpc", message: { command, reqid, data } }));
        setTimeout(() => {
            if (pending.has(reqid)) { pending.delete(reqid); reject(new Error(`timeout: ${command}`)); }
        }, 8000);
    });
}

ws.on("message", (buf) => {
    let msg;
    try { msg = JSON.parse(buf.toString()); } catch { return; }
    if (msg.type === "bus:registered") { console.log(`✓ registered as ${msg.agent_id}\n`); run(); return; }
    // RPC responses arrive wrapped: { eventtype: "rpc", data: <RpcMessage> }
    const rpc = msg.eventtype === "rpc" ? msg.data : msg;
    const resid = rpc?.resid;
    if (resid && pending.has(resid)) {
        const { resolve, reject } = pending.get(resid);
        pending.delete(resid);
        if (rpc.error) reject(new Error(rpc.error));
        else resolve(rpc.data);
    }
});

ws.on("open", () => ws.send(JSON.stringify({ type: "bus:register", agent_id: AGENT })));
ws.on("error", (e) => { console.error("WS error:", e.message); process.exit(1); });

const show = (label, v) => console.log(`${label}:`, JSON.stringify(v, null, 2));

// Hit the REST surface the way agentmux-mcp does (X-AuthKey header, /api/v1/...).
async function rest(method, path, body) {
    const res = await fetch(`http://127.0.0.1:${PORT}${path}`, {
        method,
        headers: { "X-AuthKey": KEY, "Content-Type": "application/json" },
        body: body ? JSON.stringify(body) : undefined,
    });
    const text = await res.text();
    let parsed; try { parsed = JSON.parse(text); } catch { parsed = text; }
    console.log(`  ${method} ${path.split("?")[0]} → ${res.status}`, JSON.stringify(parsed, null, 2));
    if (!res.ok) throw new Error(`REST ${path} → ${res.status}`);
    return parsed;
}

async function run() {
    try {
        console.log("=== identity.self.accounts (before) ===");
        show("accounts", await call("identity.self.accounts", { agent_id: AGENT }));

        console.log("\n=== identity.account.upsert (dummy, validate=false) ===");
        const up = await call("identity.account.upsert", {
            agent_id: AGENT, provider: "anthropic", name: "AgentX test key",
            kind: "api_key", secret: "sk-ant-test-DUMMY-0123456789", validate: false,
        });
        show("upsert", up);

        console.log("\n=== identity.self.accounts (after) ===");
        show("accounts", await call("identity.self.accounts", { agent_id: AGENT }));

        console.log("\n=== identity.account.validate (ad-hoc, no store) ===");
        show("validate", await call("identity.account.validate", {
            provider: "anthropic", secret: "sk-ant-adhoc-DUMMY",
        }));

        console.log("\n=== bundle.list ===");
        show("bundles", await call("bundle.list", {}));

        console.log("\n=== bundle.self.get ===");
        show("self-bundle", await call("bundle.self.get", { agent_id: AGENT }));

        console.log("\n=== memory.write ===");
        show("write", await call("memory.write", {
            agent_id: AGENT, filename: "app-api-smoke.md",
            content: "---\nname: app-api-smoke\n---\nWritten via App API memory.write.\n",
        }));

        console.log("\n=== memory.list ===");
        show("list", await call("memory.list", { agent_id: AGENT }));

        console.log("\n=== memory.read ===");
        show("read", await call("memory.read", { agent_id: AGENT, filename: "app-api-smoke.md" }));

        console.log("\n=== S1 negative: mismatched agent_id (expect FORBIDDEN) ===");
        try {
            await call("identity.self.accounts", { agent_id: "SomeoneElse" });
            console.log("✗ NO ERROR — S1 check failed to reject!");
        } catch (e) { console.log("✓ rejected:", e.message); }

        console.log("\n=== identity.self.unlink (cleanup) ===");
        show("unlink", await call("identity.self.unlink", { agent_id: AGENT, provider: "anthropic" }));

        // ---- REST path (the MCP→REST→handler chain agentmux-mcp uses) ----
        console.log("\n=== REST /api/v1/agent/memory/list (server-stamped agent_id) ===");
        await rest("GET", `/api/v1/agent/memory/list?agent_id=${encodeURIComponent(AGENT)}`);
        console.log("\n=== REST /api/v1/agent/preset/get (self — no id/name) ===");
        await rest("GET", `/api/v1/agent/preset/get?agent_id=${encodeURIComponent(AGENT)}`);
        console.log("\n=== REST /api/v1/agent/identity/accounts ===");
        await rest("GET", `/api/v1/agent/identity/accounts?agent_id=${encodeURIComponent(AGENT)}`);
        console.log("\n=== REST memory/write → read round-trip ===");
        await rest("POST", `/api/v1/agent/memory/write`, { agent_id: AGENT, filename: "rest-smoke.md", content: "via REST\n" });
        await rest("GET", `/api/v1/agent/memory/read?agent_id=${encodeURIComponent(AGENT)}&filename=rest-smoke.md`);

        console.log("\nAll calls completed.");
        ws.close();
        process.exit(0);
    } catch (e) {
        console.error("\n✗ FAILED:", e.message);
        ws.close();
        process.exit(1);
    }
}

// Ad-hoc App API smoke test — exercises identity.*, preset.*, memory.* as AgentX.
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

        console.log("\n=== preset.list ===");
        show("presets", await call("preset.list", {}));

        console.log("\n=== preset.self.get ===");
        show("self-preset", await call("preset.self.get", { agent_id: AGENT }));

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

        console.log("\nAll calls completed.");
        ws.close();
        process.exit(0);
    } catch (e) {
        console.error("\n✗ FAILED:", e.message);
        ws.close();
        process.exit(1);
    }
}

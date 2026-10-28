// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Operator-only RPCs need the host's proof that this window's connection is the
// operator's (crates/srv/src/server/operator_proof.rs): on each (re)connect the
// window gets a challenge from srv and the host vouches for it with its own token.
// Kept free of store imports: rpc-client.ts imports it.

/** srv commands that refuse a connection the host hasn't proven
 *  (`RpcContext::operator_only` and the guards built on it). */
const OPERATOR_COMMANDS: ReadonlySet<string> = new Set([
    "agentinput",
    "agent.pane.unregister",
    "agent.setSeat",
    "agent.takeover",
    "agent.move-here",
    "blockstats.detail",
    "browser_profiles.create",
    "browser_profiles.delete",
    "browser_profiles.update",
    "fleet.broadcast",
    "history.agent_sessions",
    "history.agents",
    "history.locate",
    "history.reclaim",
    "history.usage",
    "identity.account.availability",
    "identity.account.move_agents",
    "identity.account.signed_in",
    "jobs.act",
    "jobs.save",
    "jobs.templates.approve",
    "jobs.templates.catalog",
    "jobs.templates.catalog.install",
    "jobs.templates.install",
    "jobs.templates.uninstall",
    "kanban.act",
    "pins.adopt",
    "pins.forget",
    "pins.list",
    "tower.forget",
    "tower.pair",
    "tower.peers",
    "tower.sample",
    "writeagentconfig",
]);

/** How srv words a refusal for a connection it hasn't seen proven. */
const UNPROVEN = "isn't the AgentMux window";
/** The longest an operator RPC waits for a proof before it is sent anyway. */
const PROOF_WAIT_MS = 10_000;

type OperatorProofDeps = {
    /** `operator.challenge` on the connection that just opened. */
    challenge: () => Promise<{ nonce: string }>;
    /** The host's `operator_attest`; absent when there is no host to vouch. */
    attest?: (nonce: string) => Promise<void>;
};

let generation = 0;
let current: Promise<boolean> = Promise.resolve(false);
let settleExpected: ((proven: boolean) => void) | null = null;

/** The window's connection is about to open and can be proven: operator RPCs
 *  made before then wait for that first proof. */
export function expectOperatorProof(): void {
    generation++;
    current = new Promise<boolean>((resolve) => (settleExpected = resolve));
}

/** Prove the connection that just opened. Call it on every (re)connect: a new
 *  connection starts unproven. Without a host it does nothing. */
export function proveOperatorConnection(deps: OperatorProofDeps): Promise<boolean> {
    generation++;
    const attest = deps.attest;
    const attempt = attest
        ? (async () => {
              try {
                  const { nonce } = await deps.challenge();
                  await attest(nonce);
                  return true;
              } catch (e) {
                  console.warn("operator proof: this window's connection could not be proven", e);
                  return false;
              }
          })()
        : Promise.resolve(false);
    current = attempt;
    const settle = settleExpected;
    settleExpected = null;
    if (settle) void attempt.then(settle);
    return attempt;
}

/** Resolves when the current connection's proof has settled (true if it was
 *  proven), or after `PROOF_WAIT_MS`. */
export function operatorProofReady(): Promise<boolean> {
    let timer: ReturnType<typeof setTimeout>;
    const timeout = new Promise<boolean>((resolve) => (timer = setTimeout(() => resolve(false), PROOF_WAIT_MS)));
    return Promise.race([current, timeout]).finally(() => clearTimeout(timer));
}

export function isOperatorCommand(command: string): boolean {
    return OPERATOR_COMMANDS.has(command);
}

function isUnproven(e: unknown): boolean {
    const message = e instanceof Error ? e.message : String(e);
    return message.includes(UNPROVEN);
}

/** Send an operator-only RPC once the connection's proof has settled. When srv
 *  answers that the connection isn't proven because a reconnect started a new
 *  one meanwhile, it waits for that connection's proof and sends once more.
 *  srv refuses before acting, so the second send can't repeat an action. */
export async function sendAsOperator<T>(send: () => Promise<T>): Promise<T> {
    await operatorProofReady();
    const sentAt = generation;
    try {
        return await send();
    } catch (e) {
        if (!isUnproven(e) || generation === sentAt) throw e;
        await operatorProofReady();
        return await send();
    }
}

/** Any other RPC: if srv refuses it as unproven (an operator-only command
 *  missing from OPERATOR_COMMANDS), wait for this connection's proof and, once
 *  it is proven, send it once more. srv refuses before acting, so the second
 *  send can't repeat an action; the list only saves that first round trip. */
export async function sendRetryingUnproven<T>(send: () => Promise<T>): Promise<T> {
    try {
        return await send();
    } catch (e) {
        if (!isUnproven(e)) throw e;
        if (!(await operatorProofReady())) throw e;
        return await send();
    }
}

/** Forget every proof: for tests. */
export function resetOperatorProofForTests(): void {
    generation = 0;
    current = Promise.resolve(false);
    settleExpected = null;
}

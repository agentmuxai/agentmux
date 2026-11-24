// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Operator-only RPCs need the host's proof that this window's connection is the
// operator's (crates/srv/src/server/operator_proof.rs): on each (re)connect the
// window gets a challenge from srv and the host vouches for it with its own token.
// Kept free of store imports: rpc-client.ts imports it.

/** srv commands that refuse a connection the host hasn't proven
 *  (`RpcContext::operator_only` and the guards built on it). */
export const OPERATOR_COMMANDS: ReadonlySet<string> = new Set([
    "agentanswer",
    "agentcancel",
    "agentinput",
    "agent.pane.unregister",
    "agent.setSeat",
    "agent.takeover",
    "agent.move-here",
    "audit.query",
    "blockstats.detail",
    "browser.signin_fill",
    "browser.signin_offer_answer",
    "browser.signin_stop",
    "browser.signup_password",
    "browser_profiles.create",
    "browser_profiles.delete",
    "browser_profiles.update",
    "controllerinput",
    "fleet.broadcast",
    "gpu.sample",
    "history.agent_sessions",
    "history.agents",
    "history.locate",
    "history.reclaim",
    "history.usage",
    "identity.account.availability",
    "identity.account.move_agents",
    "identity.account.signed_in",
    "identity.account.usage",
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
    "tooldecision",
    "tower.forget",
    "tower.pair",
    "tower.peers",
    "tower.sample",
    "widgets.session",
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
/** The last connection's proof deps, to prove it again when its proof lapses (a host that vouches for a lease). */
let lastDeps: OperatorProofDeps | null = null;
let reproving: Promise<boolean> | null = null;
/** A proof is expected or being made for the current connection: input that needs it waits. */
let pending = false;
let pendingSince = 0;
const settledListeners = new Set<(proven: boolean) => void>();
let refusalHandler: ((refusal: OperatorRefusal) => void) | null = null;

/** An operator-only action srv refused that the app could not put right: the person must be told. */
export type OperatorRefusal = {
    /** srv's words, "FORBIDDEN: ..." */
    message: string;
    /** The connection's proof is missing or lapsed and proving it again did not help. */
    unproven: boolean;
};

/** Where a final refusal goes so the person sees it, not only the console (the web host shows a line). */
export function setOperatorRefusalHandler(handler: ((refusal: OperatorRefusal) => void) | null): void {
    refusalHandler = handler;
}

function refuse(e: unknown): void {
    const message = e instanceof Error ? e.message : String(e);
    if (message.includes("FORBIDDEN") || message.includes(UNPROVEN)) refusalHandler?.({ message, unproven: message.includes(UNPROVEN) });
}

/** The window's connection is about to open and can be proven: operator RPCs
 *  made before then wait for that first proof. */
export function expectOperatorProof(): void {
    generation++;
    markPending();
    current = new Promise<boolean>((resolve) => (settleExpected = resolve));
}

/** Prove the connection that just opened. Call it on every (re)connect: a new
 *  connection starts unproven. Without a host it does nothing. */
export function proveOperatorConnection(deps: OperatorProofDeps): Promise<boolean> {
    generation++;
    const mine = generation;
    markPending();
    lastDeps = deps;
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
    void attempt.then((proven) => {
        if (generation === mine) settlePending(proven);
    });
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

/** This connection's proof lapsed (a host that vouches for a lease): prove it again, once at a time. False when
 *  there is no host to vouch or it would not. */
function reprove(): Promise<boolean> {
    const deps = lastDeps;
    // While a new connection's first proof is still expected, `lastDeps` is the previous connection's: proving with
    // them would settle the new connection's expectation with the old one's result.
    if (!deps?.attest || settleExpected) return Promise.resolve(false);
    reproving ??= proveOperatorConnection(deps).finally(() => (reproving = null));
    return reproving;
}

export function isOperatorCommand(command: string): boolean {
    return OPERATOR_COMMANDS.has(command);
}

/** srv's words for a refusal of a connection it has not seen proven (or whose proof lapsed). */
export function isUnprovenMessage(message: string): boolean {
    return message.includes(UNPROVEN);
}

/** The connection's proof lapsed and srv refused input for it: prove it again. False when there is no host to vouch or it would not. */
export function reproveForInput(): Promise<boolean> {
    return reprove();
}

function isUnproven(e: unknown): boolean {
    const message = e instanceof Error ? e.message : String(e);
    return message.includes(UNPROVEN);
}

/** Send an operator-only RPC once the connection's proof has settled. When srv
 *  answers that the connection isn't proven, either a reconnect started a new
 *  one meanwhile (it waits for that connection's proof) or this one's proof
 *  lapsed (it proves it again), and it sends once more. srv refuses before
 *  acting, so the second send can't repeat an action. A refusal that stands is
 *  handed to the refusal handler so the person sees it. */
export async function sendAsOperator<T>(send: () => Promise<T>): Promise<T> {
    // Nothing to wait for once the proof has settled (the resize of a pane being dragged calls this many times a second).
    if (pending) await operatorProofReady();
    const sentAt = generation;
    try {
        return await send();
    } catch (e) {
        try {
            if (!isUnproven(e)) throw e;
            if (generation !== sentAt) await operatorProofReady();
            else if (!(await reprove())) throw e;
            return await send();
        } catch (final) {
            refuse(final);
            throw final;
        }
    }
}

/** Any other RPC: if srv refuses it as unproven (an operator-only command
 *  missing from OPERATOR_COMMANDS), wait for this connection's proof and, once
 *  it is proven, send it once more. srv refuses before acting, so the second
 *  send can't repeat an action; the list only saves that first round trip. */
export async function sendRetryingUnproven<T>(send: () => Promise<T>): Promise<T> {
    const sentAt = generation;
    try {
        return await send();
    } catch (e) {
        if (!isUnproven(e)) throw e;
        try {
            // A reconnect since the send: wait for its proof. The same connection: its proof lapsed, so prove it again.
            const proven = generation !== sentAt ? await operatorProofReady() : await reprove();
            if (!proven) throw e;
            return await send();
        } catch (final) {
            refuse(final);
            throw final;
        }
    }
}

function markPending(): void {
    if (!pending) pendingSince = performance.now();
    pending = true;
}

function settlePending(proven: boolean): void {
    pending = false;
    // The time from the connection opening to its proof, for the record of what the wait costs a keystroke.
    console.log(`[operator-proof] connection ${proven ? "proven" : "not proven"} after ${Math.round(performance.now() - pendingSince)} ms`);
    for (const listener of [...settledListeners]) listener(proven);
}

/** A proof is expected or in the making for the current connection (srv refuses the window's input until it lands). */
export function operatorProofPending(): boolean {
    return pending;
}

/** Called each time a proof settles, proven or not; returns the way to stop listening. */
export function onOperatorProofSettled(listener: (proven: boolean) => void): () => void {
    settledListeners.add(listener);
    return () => settledListeners.delete(listener);
}

/** Forget every proof: for tests. */
export function resetOperatorProofForTests(): void {
    pending = false;
    pendingSince = 0;
    settledListeners.clear();
    generation = 0;
    current = Promise.resolve(false);
    settleExpected = null;
    lastDeps = null;
    reproving = null;
    refusalHandler = null;
}

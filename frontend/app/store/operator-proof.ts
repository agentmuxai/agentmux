// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Operator-only RPCs need the host's proof that this window's connection is the
// operator's (crates/srv/src/server/operator_proof.rs): on each (re)connect the
// window gets a challenge from srv and the host vouches for it with its own token.
// Kept free of store imports: rpc-client.ts imports it. Which commands wait for the proof is srv's own decision
// (`OPERATOR_COMMANDS`, generated from crates/srv-core/src/backend/rpc/action_access.rs): every command that is not open.

import { EVENT_COMMANDS, OPEN_EVENTS, OPERATOR_COMMANDS } from "./operator-commands.generated";

/** How srv words a refusal for a connection it hasn't seen proven. */
const UNPROVEN = "isn't the AgentMux window";
/** The longest an operator RPC waits for a proof before it is sent anyway. */
const PROOF_WAIT_MS = 10_000;
/** A host that vouches for a lease (an external host: five minutes, six at most) is asked again this often, well
 *  before the lease lapses, so the window's keystrokes are never refused for it. */
const PROOF_RENEW_MS = 3 * 60_000;

type OperatorProofDeps = {
    /** `operator.challenge` on the connection that just opened. */
    challenge: () => Promise<{ nonce: string }>;
    /** The host's `operator_attest`; absent when there is no host to vouch. */
    attest?: (nonce: string) => Promise<void>;
    /** Whether the connection is still open: a renewal for a closed one is skipped (the next connect proves its own). */
    connected?: () => boolean;
};

let generation = 0;
let current: Promise<boolean> = Promise.resolve(false);
/** The current connection's proof has settled (given or refused); false while it is awaited. */
let proofSettled = true;
let settleExpected: ((proven: boolean) => void) | null = null;
/** The last connection's proof deps, to prove it again when its proof lapses (a host that vouches for a lease). */
let lastDeps: OperatorProofDeps | null = null;
let reproving: Promise<boolean> | null = null;
let renewTimer: ReturnType<typeof setTimeout> | null = null;
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
    proofSettled = false;
    current = new Promise<boolean>((resolve) => (settleExpected = resolve));
}

/** Prove the connection that just opened. Call it on every (re)connect: a new
 *  connection starts unproven. Without a host it does nothing. */
export function proveOperatorConnection(deps: OperatorProofDeps): Promise<boolean> {
    generation++;
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
    proofSettled = false;
    void attempt.then(() => {
        if (current === attempt) proofSettled = true;
    });
    const settle = settleExpected;
    settleExpected = null;
    if (settle) void attempt.then(settle);
    if (renewTimer) clearTimeout(renewTimer);
    renewTimer = null;
    if (attest) void attempt.then((proven) => proven && current === attempt && scheduleRenewal(deps));
    return attempt;
}

/** Prove the connection again before its lease lapses. Stops when the connection has closed or a proof fails. */
function scheduleRenewal(deps: OperatorProofDeps): void {
    renewTimer = setTimeout(() => {
        renewTimer = null;
        if (lastDeps === deps && deps.connected?.() !== false) void reprove();
    }, PROOF_RENEW_MS);
}

/** Resolves when the current connection's proof has settled (true if it was
 *  proven), or after `PROOF_WAIT_MS`. */
export function operatorProofReady(): Promise<boolean> {
    if (proofSettled) return current;
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

/** Whether the current connection's proof has settled: what the window's own keystrokes and resizes
 *  (`blockinput`, `setblocktermsize`, not RPCs) wait for, since srv refuses them to a connection it has not seen proven. */
export function isOperatorProofSettled(): boolean {
    return proofSettled;
}

export function isOperatorCommand(command: string): boolean {
    return OPERATOR_COMMANDS.has(command);
}

/** Whether this call waits for the proof: a command only the window may call, or one that names an event (a
 *  subscription, a history read) other than the few srv opens to every connection. */
export function waitsForProof(command: string, data: unknown): boolean {
    if (OPERATOR_COMMANDS.has(command)) return true;
    if (!EVENT_COMMANDS.has(command)) return false;
    const event = (data as { event?: unknown } | null | undefined)?.event;
    return !(typeof event === "string" && OPEN_EVENTS.has(event));
}

/** srv refused one of the window's own keystrokes or resizes (a `bus:error` frame, not an RPC answer) because this
 *  connection's proof had lapsed: prove it again, so the input that follows is held until it has. The refused input
 *  itself is not repeated. Anything else in the frame is not this module's. */
export function reproveAfterRefusedInput(error: unknown): void {
    if (isUnproven(error)) void reprove();
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
    await operatorProofReady();
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

/** Forget every proof: for tests. */
export function resetOperatorProofForTests(): void {
    generation = 0;
    proofSettled = true;
    current = Promise.resolve(false);
    settleExpected = null;
    lastDeps = null;
    reproving = null;
    if (renewTimer) clearTimeout(renewTimer);
    renewTimer = null;
    refusalHandler = null;
}

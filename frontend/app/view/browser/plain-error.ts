// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A failure of a call from the browser view to AgentMux, in words for the person.
 *
 * The host already says "couldn't reach AgentMux" for the calls it makes to srv
 * (`client/srv_failure.rs`); this is the net under the rest: anything that still
 * carries the operating system's or the network stack's own words (a banner used to
 * show "read from srv: A connection attempt failed … (os error 10060)").
 */

// The request never went out: the connection could not be made.
const NOT_SENT = /\bconnect(ing)? to srv\b|ECONNREFUSED|ENOTFOUND|EHOSTUNREACH|ENETUNREACH|actively refused/i;
// The request may have gone out and got no answer, or the connection broke. A browser's
// "Failed to fetch" is the same whether the POST never left or only its answer was lost.
const NO_ANSWER =
    /connection attempt failed|connected party did not properly respond|\b(read|write)(ing)? (from|to) srv\b|ECONN\w+|ETIMEDOUT|EPIPE|failed to fetch|networkerror|network request failed|socket hang up/i;

/** The host already worded a failure to reach srv for the person (`client/srv_failure.rs`). */
const WORDED = /^(Couldn't reach AgentMux|AgentMux didn't answer)/;

/** Whether `message` is the system's own wording for a connection that failed. */
export function isRawNetworkFailure(message: string): boolean {
    return NOT_SENT.test(message) || NO_ANSWER.test(message);
}

/** The plain sentence for a call that could not reach AgentMux. `what` completes
 *  "Couldn't reach AgentMux to …" (e.g. "turn on auto-approve"). `sent`: the request
 *  may have gone out, so whether it was done is not known. */
export function unreachableText(what: string, sent = false): string {
    return sent
        ? `AgentMux didn't answer in time when asked to ${what}, so it isn't certain it was done. Check this pane before trying again.`
        : `Couldn't reach AgentMux to ${what}. Nothing was changed. Try again in a moment.`;
}

/**
 * `err` for the person. A failure to reach AgentMux becomes `unreachableText(what)`;
 * any other message (srv's own reason, already worded for the person) is shown as it is,
 * through `wrap` when the caller puts a lead-in before it.
 */
export function plainError(err: unknown, what: string, wrap: (message: string) => string = (m) => m): string {
    const message = String((err as { message?: unknown } | null | undefined)?.message ?? err ?? "").trim();
    if (!message) return `Couldn't ${what}.`;
    if (WORDED.test(message)) return message;
    if (NOT_SENT.test(message)) return unreachableText(what);
    return NO_ANSWER.test(message) ? unreachableText(what, true) : wrap(message);
}

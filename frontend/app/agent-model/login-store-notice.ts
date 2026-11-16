// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the sign-in says when it got as far as the code but the sign-in could not be kept, so the person is never
 * told they were too slow. `no-keychain`: macOS has no login keychain for this account (found before the sign-in is
 * attempted). `unfinished`: the code was sent and the sign-in never finished.
 */

export type NotStoredWhy = "no-keychain" | "unfinished";

/** macOS's own dialog: the safe answer is Cancel. Reset To Defaults would change the person's keychain settings. */
const DIALOG_ADVICE = "If macOS shows a Keychain dialog, choose Cancel, never Reset To Defaults.";

export function notStoredNotice(why: NotStoredWhy): string {
    switch (why) {
        case "no-keychain":
            return (
                "macOS has no login keychain for this account, so it can't keep a sign-in here. " +
                "Quit AgentMux and start it from your usual macOS login, without a changed HOME, then sign in again. " +
                DIALOG_ADVICE
            );
        case "unfinished":
            return (
                "The code was sent, but the sign-in didn't finish. It may be waiting on a dialog from macOS. " +
                DIALOG_ADVICE +
                " Then sign in again."
            );
    }
}

/** The keychain problem code srv reports (`CheckCliAuthResult.store_problem`) as the reason, if it names one. */
export function whyFromStoreProblem(problem: string | null | undefined): NotStoredWhy | undefined {
    return problem === "no-default-keychain" ? "no-keychain" : undefined;
}

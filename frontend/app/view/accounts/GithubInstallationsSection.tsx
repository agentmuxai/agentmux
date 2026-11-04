// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A brokered GitHub account's installations in its details: where the AgentMux GitHub App is
// installed for this account, Install on repositories (GitHub's page in the browser, then the new
// installation is linked here), Check for installations, and Unlink
// (Connect with GitHub, SPEC_CONNECT_WITH_GITHUB §4.2, §5.7).

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import {
    callInstallations,
    linkedLine,
    NO_INSTALLATIONS_HINT,
    problemLine,
    reposLine,
    SYNC_EVERY_MS,
    waitingGivesUp,
    type GithubInstallations,
} from "./github-installations";

export function GithubInstallationsSection(): JSX.Element {
    const [data, setData] = createSignal<GithubInstallations | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [note, setNote] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    const [waitingSince, setWaitingSince] = createSignal<number | null>(null);
    let timer: ReturnType<typeof setInterval> | undefined;

    const stopWaiting = () => {
        if (timer) clearInterval(timer);
        timer = undefined;
        setWaitingSince(null);
    };
    onCleanup(stopWaiting);

    // What a call answered: the rows, the error in words, and a note when it linked something.
    const take = (answer: GithubInstallations) => {
        setData(answer);
        setError(answer.signedIn ? (answer.error ?? null) : "Sign in to AgentMux Cloud to see the installations.");
        const line = linkedLine(answer.linked, answer.installations);
        if (line) {
            setNote(line);
            stopWaiting();
        }
    };
    const call = async (action: Parameters<typeof callInstallations>[0], installationId?: string) => {
        setBusy(true);
        try {
            take(await callInstallations(action, installationId));
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };
    onMount(() => void call("list"));

    const install = async () => {
        setNote(null);
        await call("start");
        if (error()) return;
        // A second click while waiting replaces the first timer rather than leaving it running.
        stopWaiting();
        const started = Date.now();
        setWaitingSince(started);
        timer = setInterval(() => {
            if (waitingGivesUp(started, Date.now())) return stopWaiting();
            void callInstallations("sync").then(take, () => {});
        }, SYNC_EVERY_MS);
    };
    const unlink = async (id: string) => {
        setNote(null);
        await call("unlink", id);
    };
    const rows = () => data()?.installations ?? [];

    return (
        <>
            <div class="identity-detail-section">Installations</div>
            <For each={rows()}>
                {(row) => (
                    <div class="identity-detail-field">
                        <span class="identity-detail-label">@{row.accountLogin}</span>
                        <span class="identity-detail-value">
                            {reposLine(row)}
                            <Show when={problemLine(row)}>{(why) => <> · {why()}</>}</Show>
                        </span>
                        <Button density="compact" icon="link-slash" disabled={busy()} onClick={() => void unlink(row.installationId)}>
                            Unlink
                        </Button>
                    </div>
                )}
            </For>
            <Show when={data() && rows().length === 0 && !error()}>
                <div class="identity-detail-hint">{NO_INSTALLATIONS_HINT}</div>
            </Show>
            <Show when={waitingSince() !== null}>
                <div class="identity-detail-hint">Waiting for you to finish on GitHub. This list updates by itself.</div>
            </Show>
            <Show when={note()}>{(line) => <div class="identity-detail-hint">{line()}</div>}</Show>
            <Show when={error()}>
                {(e) => (
                    <div class="identity-form-error" role="alert">
                        {e()}
                    </div>
                )}
            </Show>
            <div class="identity-key-actions">
                <Button tone="accent" icon="arrow-up-right-from-square" disabled={busy()} onClick={() => void install()}>
                    Install on repositories…
                </Button>
                <Button icon="rotate-right" disabled={busy()} onClick={() => void call("refresh")}>
                    Check for installations
                </Button>
            </div>
        </>
    );
}

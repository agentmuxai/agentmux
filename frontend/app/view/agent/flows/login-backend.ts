// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The primitives a provider-CLI login is built from (start it and capture its
// URL, deliver a pasted code, report on it, cancel it, open a terminal for
// it), and who runs them: the desktop host (`runCliLogin` and friends), or
// srv (`auth.*`) for a host that can't (`HostCaps.hostLogin` false). The login
// flows (run-provider-login.ts and its callers) only talk to this.
// docs/specs/SPEC_PROVIDER_LOGIN_THROUGH_SRV_2026_10_09.md, phase L4.

import { getApi } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { sleep } from "@/util/util";

export interface LoginStart {
    providerId: string;
    cliPath: string;
    loginArgs: string[];
    /** The CLI's own auth check; srv runs it to confirm a login. */
    checkArgs: string[];
    authEnv: Record<string, string>;
    requiresTty: boolean;
    authConfigDirEnvVar: string;
}

export interface LoginStatus {
    /** The login process is still running. */
    active: boolean;
    /** This login produced credentials. */
    credential_changed: boolean;
    /** Bumped by every `start`: a different value means a newer login replaced
     *  the one a caller started. */
    generation: number;
}

export interface LoginBackend {
    /** Start a login; resolves with the URL to open, or null if none appeared
     *  in the capture window. The login keeps running for `submitCode`. */
    start(p: LoginStart): Promise<string | null>;
    /** Deliver a pasted code (or callback URL) to the running login. */
    submitCode(providerId: string, code: string): Promise<void>;
    status(): Promise<LoginStatus>;
    /** Stop the running login, if any. */
    cancel(): Promise<void>;
    /** Run the login in a terminal the user drives. */
    openTerminal(cliPath: string, loginArgs: string[], env: Record<string, string>): Promise<void>;
}

/** The desktop host's commands. */
const hostBackend: LoginBackend = {
    start: (p) => getApi().runCliLogin(p.cliPath, p.loginArgs, p.authEnv, p.requiresTty, p.authConfigDirEnvVar),
    submitCode: (providerId, code) => getApi().setProviderAuth(providerId, code),
    status: () => getApi().getCliLoginStatus(),
    cancel: () => getApi().cancelCliLogin(),
    openTerminal: async (cliPath, loginArgs, env) => {
        await getApi().openLoginTerminal(cliPath, loginArgs, env);
    },
};

/** How long `start` waits for a URL, as the host's capture does. */
const URL_CAPTURE_MS = 15_000;
const URL_POLL_MS = 400;

/** srv's `auth.*`: one login at a time, like the host's single slot. */
function makeSrvBackend(rpc = RpcApi, client = TabRpcClient): LoginBackend {
    let sessionId: string | null = null;
    let generation = 0;

    const cancel = async () => {
        const id = sessionId;
        sessionId = null;
        if (id) await rpc.AuthCancelCommand(client, { sessionId: id }).catch(() => {});
    };

    return {
        async start(p) {
            await cancel();
            generation += 1;
            // Not `directAccount`: the flow has already made the account's
            // directory (it's in authEnv) and saves the account itself, as it
            // does after a host login.
            const { sessionId: id, authUrl } = await rpc.AuthStartCommand(client, {
                providerId: p.providerId,
                cliPath: p.cliPath,
                authLoginArgs: p.loginArgs,
                authCheckArgs: p.checkArgs,
                authEnv: p.authEnv,
                requiresTty: p.requiresTty,
            });
            sessionId = id;
            if (authUrl) return authUrl;
            const deadline = Date.now() + URL_CAPTURE_MS;
            while (Date.now() < deadline && sessionId === id) {
                await sleep(URL_POLL_MS);
                const s = await rpc.AuthPollCommand(client, { sessionId: id });
                if (s.status === "url-available") return s.authUrl;
                if (s.status === "code-emitted") return s.verificationUrl;
                if (s.status === "success" || s.status === "failed") return null;
            }
            return null;
        },
        async submitCode(_providerId, code) {
            if (!sessionId) throw new Error("no sign-in is running");
            const r = await rpc.AuthSubmitCallbackCommand(client, { sessionId, callbackUrl: code });
            if (!r.success) throw new Error(r.error ?? "the sign-in didn't accept the code");
        },
        async status() {
            if (!sessionId) return { active: false, credential_changed: false, generation };
            const s = await rpc.AuthPollCommand(client, { sessionId });
            const done = s.status === "success" || s.status === "failed";
            return { active: !done, credential_changed: s.status === "success", generation };
        },
        cancel,
        async openTerminal() {
            // A terminal pane running the login is phase L3 of the spec.
            throw new Error("a login terminal isn't available here yet");
        },
    };
}

const srvBackend = makeSrvBackend();

/**
 * The host's login unless the host says it has none (`hostLogin: false`). "No
 * host yet" keeps the host's, as before this seam existed: the login flows
 * only run once the host is up.
 */
export function loginBackend(): LoginBackend {
    return window.api?.getHostCaps?.().hostLogin === false ? srvBackend : hostBackend;
}

export const __test = { makeSrvBackend, hostBackend };

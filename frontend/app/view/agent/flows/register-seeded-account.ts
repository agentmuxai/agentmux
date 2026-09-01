// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Account-dir minting + account persistence for the provider login flow.
 *
 * PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7 ("single point, not
 * global"): `identity/resolver.rs`'s layer-3 spawn gate unconditionally
 * requires a real bound account for any oauth-class provider — the
 * `use_ambient_login` escape hatch that used to let a shared-dir credential
 * work invisibly (no Armory row, no expiry tracking, no way to tell it apart
 * from a genuinely unconfigured agent) is retired. So a completed login has to
 * do two things, not one:
 *
 *   1. Mint a per-account isolated dir (`ensureAccountDir` — a thin wrapper
 *      around the `identity.ensureaccountdir` RPC, itself the same
 *      `compute_and_ensure_account_dir` `auth.start` already uses for a
 *      real OAuth handshake).
 *   2. Persist the account row (`persistSeededAccount`) so Armory can see it,
 *      the expiry probe can track it, and the resolver's gate can find it.
 *
 * Exported as separate steps because the terminal-login tier needs the dir
 * minted BEFORE it opens a terminal (to know where to point the login), but
 * can only persist AFTER the user finishes and a poll detects the credential.
 *
 * The middle step this module used to have — "seed a valid credential into
 * that dir" from the operator's personal `~/.claude` — was removed 2026-08-31
 * (per-channel auth enforcement); the credential now only ever arrives from a
 * real login the user completes in this channel. The names still say "seeded"
 * for continuity with the RPCs and call sites, but nothing is copied from
 * anywhere.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { LogFn } from "../types";

export interface MintedAccountDir {
    accountId: string;
    dir: string;
}

/** Step 1: mint (or resolve, when `existingAccountId` is set) a per-account
 *  isolated dir. Returns `null` if the provider isn't oauth-class or the
 *  dir couldn't be created. */
export async function ensureAccountDir(
    providerId: string,
    log: LogFn,
    existingAccountId?: string,
): Promise<MintedAccountDir | null> {
    try {
        const mint = await RpcApi.EnsureAccountDirCommand(TabRpcClient, {
            providerId,
            existingAccountId,
        });
        if (!mint.dir) {
            log("auth", `${providerId} has no isolated config dir to seed into`, "error");
            return null;
        }
        return { accountId: mint.accountId, dir: mint.dir };
    } catch (e: any) {
        log("auth", `couldn't allocate an account directory: ${e?.message ?? String(e)}`, "error");
        return null;
    }
}

/** Step 3: persist the IdentityAccount row once a valid credential is
 *  confirmed sitting in `dir`. */
export async function persistSeededAccount(
    providerId: string,
    accountId: string,
    dir: string,
    log: LogFn,
): Promise<boolean> {
    try {
        await RpcApi.UpsertIdentityAccountCommand(TabRpcClient, {
            id: accountId,
            name: `${providerId}-oauth`,
            provider: providerId,
            kind: "oauth",
            secret_ref: { backend: "oauth_config_dir", dir },
            status: "valid",
        });
        log("auth", "registered a real Armory account from your existing login");
        return true;
    } catch (e: any) {
        log(
            "auth",
            `credential seeded but the Armory account couldn't be registered: ${e?.message ?? String(e)}`,
            "error",
        );
        return false;
    }
}

// `registerSeededAccount` (steps 1+2+3 composed) REMOVED 2026-08-31 along with
// the tier-2 "seed from global" flow it existed to serve — it copied the
// operator's personal ~/.claude credential into a minted account dir, which
// defeated per-channel isolation. See
// docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md #3.
//
// `ensureAccountDir` and `persistSeededAccount` above are UNCHANGED and still
// load-bearing: the terminal-login tier mints a dir before opening the terminal
// and persists the account after the user completes a real OAuth in it.

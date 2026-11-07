// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Export a computer-use skill as one `.skill` file (docs/specs/SPEC_PACKS_2026_11_05.md §5): srv writes it to
 * the Downloads folder, so there is no save dialog, and says where it went.
 */

import { createSignal, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

export function ExportSkill(p: { skillId: string }): JSX.Element {
    const [busy, setBusy] = createSignal(false);
    const [saved, setSaved] = createSignal("");
    const [error, setError] = createSignal("");

    async function run(): Promise<void> {
        setBusy(true);
        setError("");
        setSaved("");
        try {
            const r = await RpcApi.SkillCatalogExportCommand(TabRpcClient, { skill_id: p.skillId });
            setSaved(`Saved to ${r.path}, ${r.signed ? "with its publisher's signature" : "without a signature"}.`);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    }

    return (
        <div class="skill-app-export">
            <Button icon="file-export" busy={busy()} onClick={() => void run()}>
                Export as a .skill file
            </Button>
            <Show when={saved()}>
                <p class="skill-app-line skill-app-muted" role="status">
                    {saved()}
                </p>
            </Show>
            <Show when={error()}>
                <p class="skill-app-line skill-app-warn" role="alert">
                    {error()}
                </p>
            </Show>
        </div>
    );
}

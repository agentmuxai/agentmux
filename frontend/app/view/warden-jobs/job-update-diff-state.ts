// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The job update diff's state: the diff the model offers, the error and busy flag, the template under review,
// and the load, accept, review and approve calls that move between them.

import { createSignal, onMount } from "solid-js";

import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";
import type { JobView } from "@/types/rpc/JobView";
import type { TemplateUpdateDiff } from "@/types/rpc/TemplateUpdateDiff";

import type { JobsModel } from "./jobs-model";

export type JobUpdateDiffProps = { job: JobView; model: JobsModel; onClose: () => void };

export type UpdateDiffState = ReturnType<typeof createUpdateDiff>;

export function createUpdateDiff(props: JobUpdateDiffProps) {
    const [diff, setDiff] = createSignal<TemplateUpdateDiff | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    const [approving, setApproving] = createSignal<JobTemplateInfo | null>(null);

    const load = async () => {
        try {
            setDiff(await props.model.updateDiff(props.job.id));
            setError(null);
        } catch (e) {
            setError(String(e));
        }
    };
    onMount(() => void load());

    const accept = async () => {
        const d = diff();
        if (!d) return;
        setBusy(true);
        setError(null);
        try {
            const res = await props.model.save({
                job_id: props.job.id,
                definition: { template: { id: d.template_id, version: d.to_version, hash: d.to_hash } },
            });
            if (res.ok) props.onClose();
            else setError(res.error ?? "It wasn't saved.");
        } catch (e) {
            setError(String(e));
        } finally {
            setBusy(false);
        }
    };

    const review = async () => {
        const d = diff();
        if (!d) return;
        try {
            setApproving((await props.model.templateDetail(d.template_id)).template);
        } catch (e) {
            setError(String(e));
        }
    };

    const approve = async () => {
        const info = approving();
        if (!info) return;
        setBusy(true);
        setError(null);
        try {
            await props.model.approveTemplate({
                id: info.id,
                hash: info.hash,
                signer: info.signature.fingerprint ?? "",
            });
            setApproving(null);
            await load();
        } catch (e) {
            setError(String(e));
        } finally {
            setBusy(false);
        }
    };

    const cancelReview = () => setApproving(null);

    return { diff, error, busy, approving, accept, review, approve, cancelReview };
}

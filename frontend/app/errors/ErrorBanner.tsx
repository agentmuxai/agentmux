// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * <ErrorBanner /> — the canonical AgentMux error frame.
 *
 * Renders the translated title + message + recovery hint, plus a small
 * collapsed *Details* disclosure for power users who want the raw
 * backend message and the AMX code (for support requests).
 *
 * Three usage patterns:
 *
 *   1. From a known wire object:
 *        <ErrorBanner error={errPayload} />
 *      where `errPayload` matches `{ code, message?, details? }`.
 *
 *   2. From a caught exception:
 *        try { ... } catch (e) {
 *            return <ErrorBanner error={e} />;
 *        }
 *
 *   3. From a free-text string:
 *        <ErrorBanner error="install timed out" />
 *      — renders as `AMX-LEGACY`.
 *
 * The translator handles all three uniformly.
 */

import { createMemo, createSignal, Show, type JSX } from "solid-js";
import { translateError } from "./translate";
import "./ErrorBanner.scss";

interface ErrorBannerProps {
    /** Any of: wire-format object, Error instance, or string. */
    error: unknown;
    /** Optional dismiss handler — renders an × close button when set. */
    onDismiss?: () => void;
}

export const ErrorBanner = (props: ErrorBannerProps): JSX.Element => {
    // Memo so the 5+ `t()` reads per render don't re-translate; the
    // memo only re-runs when `props.error` actually changes.
    const t = createMemo(() => translateError(props.error));
    const [detailsOpen, setDetailsOpen] = createSignal(false);

    return (
        <div class="amx-error-banner" role="alert">
            <span class="amx-error-banner-icon" aria-hidden="true">⚠</span>
            <div class="amx-error-banner-body">
                <div class="amx-error-banner-title">{t().title}</div>
                <div class="amx-error-banner-message">{t().message}</div>
                <Show when={t().retry}>
                    <div class="amx-error-banner-retry">{t().retry}</div>
                </Show>
                <Show when={t().rawMessage && t().rawMessage !== t().message}>
                    <button
                        type="button"
                        class="amx-error-banner-details-toggle"
                        onClick={() => setDetailsOpen((o) => !o)}
                    >
                        {detailsOpen() ? "Hide details" : "Show details"}
                    </button>
                    <Show when={detailsOpen()}>
                        <pre class="amx-error-banner-raw">{t().rawMessage}</pre>
                    </Show>
                </Show>
            </div>
            <code class="amx-error-banner-code" title="Error code (include in bug reports)">
                {t().code}
            </code>
            <Show when={props.onDismiss}>
                <button
                    type="button"
                    class="amx-error-banner-dismiss"
                    onClick={() => props.onDismiss?.()}
                    aria-label="Dismiss"
                >
                    ×
                </button>
            </Show>
        </div>
    );
};

ErrorBanner.displayName = "ErrorBanner";

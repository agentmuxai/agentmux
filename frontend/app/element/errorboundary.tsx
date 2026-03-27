// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ErrorBoundary as SolidErrorBoundary, JSX } from "solid-js";

interface ErrorBoundaryProps {
    children?: JSX.Element;
    fallback?: JSX.Element | ((err: Error) => JSX.Element);
}

export function ErrorBoundary(props: ErrorBoundaryProps): JSX.Element {
    return (
        <SolidErrorBoundary
            fallback={(err) => {
                if (props.fallback != null) {
                    if (typeof props.fallback === "function") {
                        return (props.fallback as (err: Error) => JSX.Element)(err);
                    }
                    return props.fallback as JSX.Element;
                }
                const errorMsg = `Error: ${err?.message}\n\n${err?.stack}`;
                return <pre class="error-boundary">{errorMsg}</pre>;
            }}
        >
            {props.children}
        </SolidErrorBoundary>
    );
}

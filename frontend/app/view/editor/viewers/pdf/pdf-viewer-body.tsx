// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The PDF viewer's body: the pages pdf.js draws into, and the message over them while loading, on an error or for a password. */

import { Button, TextInput } from "@/app/element/ui";
import { Show, type JSX } from "solid-js";
import type { PdfViewerState } from "./pdf-viewer-state";

export function PdfViewerBody(props: { s: PdfViewerState }): JSX.Element {
    const s = props.s;
    return (
        <div class="pdf-viewer-body">
            <div ref={(el) => (s.container = el)} class="pdf-viewer-container" data-zoom-scope="content" classList={{ "pdf-viewer-inverted": s.invert() }} tabIndex={0} aria-label="PDF pages">
                <div ref={(el) => (s.viewerEl = el)} class="pdfViewer" />
            </div>
            <Show when={s.status() === "loading"}>
                <div class="editor-viewer-message">Loading…</div>
            </Show>
            <Show when={s.status() === "error"}>
                <div class="editor-viewer-message" role="alert">
                    Couldn't open this PDF: {s.error()}
                </div>
            </Show>
            <Show when={s.status() === "password"}>
                <form
                    class="editor-viewer-message pdf-viewer-password"
                    onSubmit={(e) => {
                        e.preventDefault();
                        const pw = s.passwordInput?.value ?? "";
                        s.setStatus("loading");
                        s.sendPassword?.(pw);
                    }}
                >
                    <div>{s.wrongPassword() ? "That password didn't open it. Try again:" : "This PDF is protected. Its password:"}</div>
                    <TextInput variant="plain" ref={(el) => (s.passwordInput = el)} type="password" aria-label="PDF password" autocomplete="off" />
                    <Button type="submit" tone="accent">
                        Open
                    </Button>
                </form>
            </Show>
        </div>
    );
}

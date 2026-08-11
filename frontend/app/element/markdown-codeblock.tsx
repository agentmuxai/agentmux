// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { CopyButton } from "@/app/element/copybutton";
import { ErrorBoundary } from "@/app/element/errorboundary";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { IconButton } from "./iconbutton";
import { Mermaid, MermaidErrorFallback } from "./markdown-mermaid";

const Code = ({ className = "", children }: { className?: string; children: any }) => {
    if (/\blanguage-mermaid\b/.test(className)) {
        const text = Array.isArray(children) ? children.join("") : String(children ?? "");
        return (
            <ErrorBoundary fallback={<MermaidErrorFallback chart={text} />}>
                <Mermaid chart={text} />
            </ErrorBoundary>
        );
    }
    return <code class={className}>{children}</code>;
};

type CodeBlockProps = {
    children: any;
    onClickExecute?: (cmd: string) => void;
};

const CodeBlock = ({ children, onClickExecute }: CodeBlockProps) => {
    // `children` here is a real DOM node, not a React-style {props:
    // {children}} tree — this markdown renderer builds elements via
    // solid-js/h/jsx-runtime (hast-util-to-jsx-runtime), which creates
    // actual DOM eagerly rather than a virtual-DOM descriptor. Walking
    // .props.children never matches, so the only reliable way to get the
    // rendered text is to read it back off the DOM via a ref.
    let contentRef: HTMLDivElement | undefined;

    const getTextContent = (): string => {
        return (contentRef?.textContent ?? "").replace(/\n$/, "");
    };

    const handleCopy = async (e: MouseEvent) => {
        await clipboardWriteText(getTextContent());
    };

    const handleExecute = (e: MouseEvent) => {
        if (onClickExecute) {
            onClickExecute(getTextContent());
        }
    };

    return (
        <pre class="codeblock">
            <div class="codeblock-content" ref={contentRef}>
                {children}
            </div>
            <div class="codeblock-actions">
                <CopyButton onClick={handleCopy} title="Copy" />
                {onClickExecute && (
                    <IconButton
                        decl={{
                            elemtype: "iconbutton",
                            icon: "regular@square-terminal",
                            click: handleExecute,
                        }}
                    />
                )}
            </div>
        </pre>
    );
};

export { Code, CodeBlock };

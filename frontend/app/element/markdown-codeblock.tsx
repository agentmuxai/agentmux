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
    const getTextContent = (children: any): string => {
        if (typeof children === "string") {
            return children;
        } else if (Array.isArray(children)) {
            return children.map(getTextContent).join("");
        } else if (children && children.props && children.props.children) {
            return getTextContent(children.props.children);
        }
        return "";
    };

    const handleCopy = async (e: MouseEvent) => {
        let textToCopy = getTextContent(children);
        textToCopy = textToCopy.replace(/\n$/, "");
        await clipboardWriteText(textToCopy);
    };

    const handleExecute = (e: MouseEvent) => {
        let textToCopy = getTextContent(children);
        textToCopy = textToCopy.replace(/\n$/, "");
        if (onClickExecute) {
            onClickExecute(textToCopy);
        }
    };

    return (
        <pre class="codeblock">
            {children}
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

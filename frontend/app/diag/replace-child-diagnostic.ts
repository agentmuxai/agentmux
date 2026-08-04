// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! replace-child-diagnostic — name the component behind the intractable
//! `replaceChild: node not a child` SolidJS reconcileArrays crash.
//!
//! The agent pane has crashed with this error across v0.43.x–v0.44.1 (block
//! aa6a64f; issue #1326; SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX §7.4).
//! Four keying fixes (#1319/#1322/#1327 + the Switch fix) did not close it, and
//! the JS stack at the throw is 100% SolidJS-internal — the offending render
//! effect runs detached from the component call stack, so the stack alone can
//! NEVER name the component.
//!
//! This installs a transparent guard on `Node.replaceChild` that, the instant
//! the node-not-a-child condition is true (i.e. one tick before SolidJS throws),
//! logs the PARENT container + the DETACHED child's tag/class/data-* + ancestry
//! + the parent's current children. That identifies the component and shows what
//! shifted underneath it. The host console bridge forwards `[fe]` logs into the
//! host log, so it lands right above the block-error-boundary render_trail.
//!
//! Cost: one `oldNode.parentNode !== this` comparison per replaceChild; the
//! (heavier) logging only runs on the anomaly, which always precedes a crash.
//! Safe to keep permanently — it never changes replaceChild's behaviour.

interface NodeDesc {
    tag: string;
    cls: string;
    id: string;
    data: Record<string, string>;
    text: string;
}

function describe(n: Node | null): NodeDesc | string | null {
    if (!n) return null;
    if (!(n instanceof Element)) {
        return `${n.nodeName}("${(n.textContent ?? "").slice(0, 40)}")`;
    }
    const data: Record<string, string> = {};
    for (const a of Array.from(n.attributes)) {
        if (a.name.startsWith("data-")) data[a.name] = a.value;
    }
    return {
        tag: n.tagName.toLowerCase(),
        cls: typeof n.className === "string" ? n.className : String(n.className ?? ""),
        id: n.id ?? "",
        data,
        text: (n.textContent ?? "").slice(0, 60),
    };
}

/** Compact tag.class[block][i] chain up to the nearest agent container. */
function ancestry(n: Node | null, max = 10): string[] {
    const out: string[] = [];
    let cur: Node | null = n;
    while (cur && out.length < max) {
        if (cur instanceof Element) {
            const cls = (typeof cur.className === "string" ? cur.className : "")
                .split(/\s+/)
                .filter(Boolean)
                .slice(0, 2)
                .join(".");
            const block = cur.getAttribute("data-block") ?? cur.getAttribute("data-block-id");
            const idx = cur.getAttribute("data-index");
            out.push(
                `${cur.tagName.toLowerCase()}${cls ? "." + cls : ""}` +
                    `${block ? `[block=${block.slice(0, 7)}]` : ""}${idx != null ? `[i=${idx}]` : ""}`,
            );
        } else {
            out.push(cur.nodeName);
        }
        cur = cur.parentNode;
    }
    return out;
}

function sampleChildren(parent: Node, max = 8): string[] {
    return Array.from(parent.childNodes)
        .slice(0, max)
        .map((k) => {
            if (!(k instanceof Element)) return k.nodeName;
            const cls = (typeof k.className === "string" ? k.className : "").split(/\s+/)[0] ?? "";
            const idx = k.getAttribute("data-index");
            return `${k.tagName.toLowerCase()}${cls ? "." + cls : ""}${idx != null ? `[i=${idx}]` : ""}`;
        });
}

let installed = false;

function installReplaceChildDiagnostic(): void {
    if (installed) return;
    if (typeof Node === "undefined" || !Node.prototype || !Node.prototype.replaceChild) return;
    installed = true;

    const orig = Node.prototype.replaceChild;
    // Match the native generic signature: replaceChild<T>(newNode, oldNode): T.
    Node.prototype.replaceChild = function <T extends Node>(
        this: Node,
        newNode: Node,
        oldNode: T,
    ): T {
        if (oldNode && oldNode.parentNode !== this) {
            try {
                const payload = {
                    parent: describe(this),
                    parentChildCount: this.childNodes.length,
                    parentChildren: sampleChildren(this),
                    parentAncestry: ancestry(this),
                    oldNode: describe(oldNode),
                    oldNodeAttachedTo: oldNode.parentNode
                        ? describe(oldNode.parentNode)
                        : "DETACHED (parentNode=null)",
                    newNode: describe(newNode),
                    stack: new Error().stack,
                };
                // Single stringified payload so the host console bridge captures
                // the whole context as one log line above the render_trail.
                // eslint-disable-next-line no-console
                console.error("[replace-child-diag] node-not-a-child (crash imminent) " + JSON.stringify(payload));
            } catch {
                // The diagnostic must never throw and mask the real error.
            }
        }
        return orig.call(this, newNode, oldNode) as T;
    };
}

// Self-install on import.
installReplaceChildDiagnostic();

// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { isBlank } from "@/util/util";
import { Subject } from "rxjs";
import { sendRawRpcMessage } from "./ws";

type MuxEventSubject = {
    handler: (event: MuxEvent) => void;
    scope?: string;
};

type MuxEventSubjectContainer = MuxEventSubject & {
    id: string;
};

type MuxEventSubscription = MuxEventSubject & {
    eventType: string;
};

type MuxEventUnsubscribe = {
    id: string;
    eventType: string;
};

// key is "eventType" or "eventType|oref"
const fileSubjects = new Map<string, SubjectWithRef<WSFileEventData>>();
const muxEventSubjects = new Map<string, MuxEventSubjectContainer[]>();

function mpsReconnectHandler() {
    for (const eventType of muxEventSubjects.keys()) {
        updateMuxEventSub(eventType);
    }
}

function makeMuxReSubCommand(eventType: string): RpcMessage {
    let subjects = muxEventSubjects.get(eventType);
    if (subjects == null) {
        return { command: "eventunsub", data: eventType };
    }
    let subreq: SubscriptionRequest = { event: eventType, scopes: [], allscopes: false };
    for (const scont of subjects) {
        if (isBlank(scont.scope)) {
            subreq.allscopes = true;
            subreq.scopes = [];
            break;
        }
        subreq.scopes.push(scont.scope);
    }
    return { command: "eventsub", data: subreq };
}

function updateMuxEventSub(eventType: string) {
    const command = makeMuxReSubCommand(eventType);
    // console.log("updateMuxEventSub", eventType, command);
    sendRawRpcMessage(command);
}

function muxEventSubscribe(...subscriptions: MuxEventSubscription[]): () => void {
    const unsubs: MuxEventUnsubscribe[] = [];
    const eventTypeSet = new Set<string>();
    for (const subscription of subscriptions) {
        // console.log("muxEventSubscribe", subscription);
        if (subscription.handler == null) {
            return;
        }
        const id: string = crypto.randomUUID();
        let subjects = muxEventSubjects.get(subscription.eventType);
        if (subjects == null) {
            subjects = [];
            muxEventSubjects.set(subscription.eventType, subjects);
        }
        const subcont: MuxEventSubjectContainer = { id, handler: subscription.handler, scope: subscription.scope };
        subjects.push(subcont);
        unsubs.push({ id, eventType: subscription.eventType });
        eventTypeSet.add(subscription.eventType);
    }
    for (const eventType of eventTypeSet) {
        updateMuxEventSub(eventType);
    }
    return () => muxEventUnsubscribe(...unsubs);
}

function muxEventUnsubscribe(...unsubscribes: MuxEventUnsubscribe[]) {
    const eventTypeSet = new Set<string>();
    for (const unsubscribe of unsubscribes) {
        let subjects = muxEventSubjects.get(unsubscribe.eventType);
        if (subjects == null) {
            continue;
        }
        const idx = subjects.findIndex((s) => s.id === unsubscribe.id);
        if (idx === -1) {
            continue;
        }
        subjects.splice(idx, 1);
        if (subjects.length === 0) {
            muxEventSubjects.delete(unsubscribe.eventType);
        }
        eventTypeSet.add(unsubscribe.eventType);
    }

    for (const eventType of eventTypeSet) {
        updateMuxEventSub(eventType);
    }
}

function getFileSubject(zoneId: string, fileName: string): SubjectWithRef<WSFileEventData> {
    const subjectKey = zoneId + "|" + fileName;
    let subject = fileSubjects.get(subjectKey);
    if (subject == null) {
        subject = new Subject<any>() as any;
        subject.refCount = 0;
        subject.release = () => {
            subject.refCount--;
            if (subject.refCount === 0) {
                subject.complete();
                fileSubjects.delete(subjectKey);
            }
        };
        fileSubjects.set(subjectKey, subject);
    }
    subject.refCount++;
    return subject;
}

// Sysinfo and blockstats are droppable 1Hz telemetry. Dispatching them
// synchronously in the WebSocket onmessage task competes with xterm.js's
// rAF-scheduled canvas render in the same paint pass, causing a 1Hz jerk
// when backspace (or any held key) is generating rapid terminal echo.
// Deferring to a new macrotask lets the current frame's rAF + paint complete
// first; sysinfo DOM updates land in their own subsequent render pass.
const DEFERRED_EVENTS = new Set(["sysinfo", "blockstats"]);

function dispatchToSubjects(event: MuxEvent) {
    const subjects = muxEventSubjects.get(event.event);
    if (subjects == null) return;
    for (const scont of subjects) {
        if (isBlank(scont.scope)) {
            scont.handler(event);
            continue;
        }
        if (event.scopes == null) continue;
        if (event.scopes.includes(scont.scope)) {
            scont.handler(event);
        }
    }
}

function handleMuxEvent(event: MuxEvent) {
    if (DEFERRED_EVENTS.has(event.event)) {
        setTimeout(() => dispatchToSubjects(event), 0);
        return;
    }
    dispatchToSubjects(event);
}

export { getFileSubject, handleMuxEvent, muxEventSubscribe, mpsReconnectHandler };

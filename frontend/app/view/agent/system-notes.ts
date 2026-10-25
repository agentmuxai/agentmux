// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMux's own notes in a user turn, split off from the message they ride on
 * (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md, P0).
 *
 * srv holds `[AgentMux] ...` notes (an upgrade, cloud messages paused, an
 * overlap) and attaches them after the next message it delivers, a blank line
 * and then one note per line (`reactive::held_notes::with_notes`). A note on
 * its own is delivered the same way. The pane used to show the whole turn as
 * one message, so a note, or a jekt with a note after it, took the human's
 * style.
 *
 * INTERIM TEXT PARSING. This reads the text because the record carries no
 * origin yet; P1 writes a typed origin and the notes list on the record, and
 * this is deleted in P5. It can only narrow: whatever it takes out of a
 * message goes to a container, never to the human style, and text it does not
 * recognise is left where it was.
 */

/** srv's marker for its own notes (`muxbus::delivery_status::notes::NOTE_MARKER`) and the space after it. */
const NOTE_MARKER = "[AgentMux] ";

/** What srv puts first on a message delivered more than two minutes after it was sent. */
const DELAY_PREFIX_RE = /^\((sent \d{1,2}:\d{2}, delivered \d{1,2}:\d{2})\) /;

export interface SplitMessage {
    /** The message without its notes: a jekt block, the human's text, ... Empty when the turn was only notes. */
    head: string;
    /** "sent 14:30, delivered 14:41" when the message came late, without the parentheses. */
    delay: string | null;
    /** Each note's text without the marker, in order. */
    notes: string[];
}

/** Split `text` into the message and the srv notes that follow it. Text with no notes comes back unchanged. */
export function splitSystemNotes(text: string): SplitMessage {
    // The prefix is srv's only in front of a jekt or a note; anywhere else it is just text.
    const found = DELAY_PREFIX_RE.exec(text);
    const delayed = found && /^\[(JEKT:|AgentMux\] )/.test(text.slice(found[0].length)) ? found : null;
    const rest = delayed ? text.slice(delayed[0].length) : text;

    let head: string;
    let tail: string;
    if (rest.startsWith("[JEKT:")) {
        // Notes come after the jekt block, never from inside it: look past its
        // closing marker, and leave anything else after the block as it was.
        const close = rest.indexOf("\n[/JEKT]");
        const end = close < 0 ? -1 : close + "\n[/JEKT]".length;
        const after = end < 0 ? "" : rest.slice(end).replace(/^\s+/, "");
        if (end < 0 || !after.startsWith(NOTE_MARKER)) {
            return { head: rest, delay: delayed ? delayed[1] : null, notes: [] };
        }
        head = rest.slice(0, end);
        tail = after;
    } else if (rest.startsWith(NOTE_MARKER)) {
        head = "";
        tail = rest;
    } else {
        const at = rest.indexOf("\n\n" + NOTE_MARKER);
        if (at < 0) return { head: rest, delay: delayed ? delayed[1] : null, notes: [] };
        head = rest.slice(0, at);
        tail = rest.slice(at + 2);
    }

    const notes: string[] = [];
    for (const line of tail.split("\n")) {
        if (line.startsWith(NOTE_MARKER)) notes.push(line.slice(NOTE_MARKER.length));
        else if (notes.length > 0) notes[notes.length - 1] += "\n" + line;
    }
    return { head, delay: delayed ? delayed[1] : null, notes: notes.map((n) => n.trimEnd()) };
}

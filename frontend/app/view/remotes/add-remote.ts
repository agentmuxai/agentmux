// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Add remote's one field: a destination as you'd type it after `ssh`
// (`user@host`, `host:port`, `[v6]:port`), split into its parts; the
// connection name it is saved under; and the plain `Host` alias it gets when
// it has to be written to ssh config.

/** The parts of a destination; empty strings where it gives none. */
export interface Destination {
    user: string;
    hostname: string;
    port: string;
}

/** Splits `user@host:port` (each part but the host optional); `null` when
 *  it isn't one. A bare IPv6 address takes no port; put it in brackets for
 *  one (`[fe80::1]:2222`). */
export function parseDestination(text: string): Destination | null {
    const s = text.trim();
    if (!s || /\s/.test(s)) return null;
    const at = s.lastIndexOf("@");
    const user = at >= 0 ? s.slice(0, at) : "";
    const rest = at >= 0 ? s.slice(at + 1) : s;
    if (at >= 0 && !user) return null;
    let hostname = rest;
    let port = "";
    const bracket = /^\[([^\]]+)\](?::(\d+))?$/.exec(rest);
    if (bracket) {
        hostname = bracket[1];
        port = bracket[2] ?? "";
    } else if ((rest.match(/:/g) ?? []).length === 1) {
        [hostname, port] = rest.split(":");
        if (!/^\d+$/.test(port)) return null;
    }
    if (!hostname) return null;
    if (port && (Number(port) < 1 || Number(port) > 65535)) return null;
    return { user, hostname, port };
}

/** The `Host` name a destination gets when none is typed: a host name's first
 *  label (`db1` for `db1.example.com`), or an address with its dots and
 *  colons as dashes, and its port, so `ssh 127.0.0.1` itself keeps meaning
 *  what it did (`127-0-0-1-2222`). */
export function defaultAlias(dest: Destination): string {
    const isAddress = /^[\d.]+$/.test(dest.hostname) || dest.hostname.includes(":");
    const base = isAddress ? dest.hostname.replace(/[.:]+/g, "-") : dest.hostname.split(".")[0];
    return dest.port && isAddress ? `${base}-${dest.port}` : base;
}

/** The remote's connection name, `user@host:port`, with port 22 when none is
 *  given and an IPv6 address in brackets: what AgentMux connects to, and the
 *  name a remote gets when none is typed. */
export function connectionName(dest: Destination): string {
    const host = dest.hostname.includes(":") ? `[${dest.hostname}]` : dest.hostname;
    return `${dest.user ? `${dest.user}@` : ""}${host}:${dest.port || "22"}`;
}

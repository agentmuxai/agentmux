// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Lists on-screen windows of processes whose owner name contains argv[1], with
// the properties that decide macOS compositing cost: layer, alpha, sharing state,
// bounds, and memory use. Uses CGWindowListCopyWindowInfo, which needs no
// Screen Recording / Accessibility permission for these fields (not window names).
import Foundation
import CoreGraphics

let needle = CommandLine.arguments.count > 1 ? CommandLine.arguments[1].lowercased() : "agentmux"
guard let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] else { exit(1) }
var rows: [String] = []
for w in list {
    let owner = (w[kCGWindowOwnerName as String] as? String) ?? ""
    if !owner.lowercased().contains(needle) { continue }
    let b = w[kCGWindowBounds as String] as? [String: Any]
    let on = (w[kCGWindowIsOnscreen as String] as? Bool) ?? false
    if !on { continue }
    let num = w[kCGWindowNumber as String] ?? "?"
    let layer = w[kCGWindowLayer as String] ?? "?"
    let alpha = w[kCGWindowAlpha as String] ?? "?"
    let store = w[kCGWindowStoreType as String] ?? "?"
    let mem = w[kCGWindowMemoryUsage as String] ?? "?"
    let pid = w[kCGWindowOwnerPID as String] ?? "?"
    rows.append("win#\(num) pid=\(pid) owner=\(owner) layer=\(layer) alpha=\(alpha) store=\(store) mem=\(mem) bounds=\(b?["Width"] ?? "?")x\(b?["Height"] ?? "?")@\(b?["X"] ?? "?"),\(b?["Y"] ?? "?")")
}
rows.forEach { print($0) }

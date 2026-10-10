// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Polls the on-screen bounds of every window owned by one process, as fast as
// CGWindowListCopyWindowInfo allows, for N seconds. No Screen Recording or
// Accessibility permission is needed for bounds (only for window titles/pixels).
// Output: one line per sample: "<epoch_ms> <id>:<x>,<y>,<w>,<h> <id>:..."
// usage: panesampler <pid> <seconds>
import Foundation
import CoreGraphics

let pid = Int32(CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "0") ?? 0
let secs = Double(CommandLine.arguments.count > 2 ? CommandLine.arguments[2] : "3") ?? 3
let end = Date().addingTimeInterval(secs)
var out = ""
var n = 0
while Date() < end {
    let t = Date().timeIntervalSince1970 * 1000
    guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] else { continue }
    var parts: [String] = []
    for w in list {
        guard (w[kCGWindowOwnerPID as String] as? Int32) == pid, let b = w[kCGWindowBounds as String] as? [String: Any] else { continue }
        let id = w[kCGWindowNumber as String] as? Int ?? 0
        let x = b["X"] as? Double ?? 0, y = b["Y"] as? Double ?? 0, wd = b["Width"] as? Double ?? 0, ht = b["Height"] as? Double ?? 0
        parts.append("\(id):\(Int(x)),\(Int(y)),\(Int(wd)),\(Int(ht))")
    }
    out += String(format: "%.1f ", t) + parts.joined(separator: " ") + "\n"
    n += 1
    usleep(2000)
}
FileHandle.standardOutput.write(out.data(using: .utf8)!)
FileHandle.standardError.write("samples=\(n)\n".data(using: .utf8)!)

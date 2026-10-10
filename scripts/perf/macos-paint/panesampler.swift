// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Polls the on-screen bounds of every window owned by one process (~300/s) until SIGTERM/SIGINT or
// <max seconds>, then prints one line per sample: "<epoch_ms> <id>:<x>,<y>,<w>,<h> ...".
// CGWindowListCopyWindowInfo needs no Screen Recording or Accessibility permission for bounds.
// usage: panesampler <pid> [max seconds]
import Foundation
import CoreGraphics

let pid = Int32(CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "0") ?? 0
let maxSecs = Double(CommandLine.arguments.count > 2 ? CommandLine.arguments[2] : "120") ?? 120
var stop = false
signal(SIGTERM) { _ in stop = true }
signal(SIGINT) { _ in stop = true }
let end = Date().addingTimeInterval(maxSecs)
var out = ""
while !stop && Date() < end {
    let t = Date().timeIntervalSince1970 * 1000
    if let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] {
        var parts: [String] = []
        for w in list {
            guard (w[kCGWindowOwnerPID as String] as? Int32) == pid, let b = w[kCGWindowBounds as String] as? [String: Any] else { continue }
            let id = w[kCGWindowNumber as String] as? Int ?? 0
            parts.append("\(id):\(Int(b["X"] as? Double ?? 0)),\(Int(b["Y"] as? Double ?? 0)),\(Int(b["Width"] as? Double ?? 0)),\(Int(b["Height"] as? Double ?? 0))")
        }
        out += String(format: "%.1f ", t) + parts.joined(separator: " ") + "\n"
    }
    usleep(3000)
}
FileHandle.standardOutput.write(out.data(using: .utf8)!)

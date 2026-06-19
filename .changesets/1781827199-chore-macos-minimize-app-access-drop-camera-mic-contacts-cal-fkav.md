---
type: minor
---

chore(macos): scope app access to the user's folder — drop camera/contacts/calendars/location/photos entitlements; keep microphone and LAN discovery as opt-in (prompt only when the user enables them via the pane mic button / status-bar LAN switch, with NSMicrophone/NSLocalNetwork usage strings); scope the editor file tree to the user's home folder

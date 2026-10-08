---
type: minor
---

Bundles no longer carry MCP servers: they belong to Connectors. On upgrade, MCP servers an agent got only through a bundle are removed (the srv log names each one), bundle exports leave them out, and importing a bundle lists its MCP servers as not imported, to add in Connectors.

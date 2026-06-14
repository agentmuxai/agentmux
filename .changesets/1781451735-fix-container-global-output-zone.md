---
type: patch
---

fix(container): thread global_output_zone through the container-exec output path (publish_line) so main compiles — a semantic merge conflict between #1399 (added the 6th arg to handle_append_block_file) and #1357 (added the publish_line call site with the old signature).

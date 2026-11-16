# Streamed browser panes: spike results (task #18044)

**Status:** analysis - results of the server-side Chromium streaming spike; the build task #18045 waits on this.

Date: 2026-11-02. Run by Parlo on the Linux host (2026-11-01 21:33 to 22:05), written up by Korp. Raw files: `spike-out.tgz` (per config: `.json`, `.h264.jsonl`, `.load`, `.sandbox.txt`, plus `run.log`). The spike harness and its pages (`text`, `scroll`, `dash`, `video`) are Parlo's.

## What was measured

Chromium 156 in a container, driven over CDP, streaming a 1280x800 viewport. Three container limits, each run for 8 s per cell (short: treat single numbers as indicative, not as a benchmark):

| Config | CPU limit | Memory limit | Peak memory seen |
|---|---|---|---|
| signedin | 1 vCPU | 4 GB | 1471 MB |
| bigger | 2 vCPU | 4 GB | 1456 MB |
| guest | 0.5 vCPU | 2 GB | 1161 MB |

Load average on the host stayed under 3.2 throughout, so the host itself was not the limit.

Experiments: E1 frame stream by page type, format, quality and device pixel ratio (dpr); E2 a `captureScreenshot` loop instead of screencast; E3 image quality (SSIM, PSNR); E4 input latency; E5 flow control (delay before acknowledging a frame); E6 several panes in one Chromium; E7 browser features through CDP; E8 client decode time; plus a separate H.264 run (ffmpeg, constant quality) per page.

## Findings

### 1. A static page costs almost nothing; a moving page is limited by CPU, not by the network

Screencast only sends a frame when the page changes. A text page produced 1 frame in 8 s: 145 kbps at JPEG 70, 1 to 4 % CPU, about 62 MB per hour.

For moving pages the bandwidth is large and the CPU limit decides the frame rate (JPEG 70, dpr 1):

| Page | 1 vCPU | 2 vCPU | 0.5 vCPU |
|---|---|---|---|
| scroll | 32 fps, 27.9 Mbps | 33 fps, 29.1 Mbps | 18 fps, 15.4 Mbps |
| dash (animated dashboard) | 42 fps, 25.3 Mbps | 60 fps, 35.7 Mbps | 18 fps, 10.7 Mbps |
| video | 19 fps, 6.7 Mbps | 28 fps, 10.0 Mbps | 7.6 fps, 2.7 Mbps |

At 0.5 vCPU the container is pinned at 50 % CPU in every moving case. At dpr 2 (a high-density screen) the size per frame rises about 2.6x (JPEG 70: 141 kB at dpr 1, 365 kB at dpr 2) and the frame rate falls: on 1 vCPU scroll and dash dropped to about 11 fps (a third), on 2 vCPU to 22 fps (two thirds), on 0.5 vCPU to 5 fps.

### 2. H.264 cuts the bandwidth by 5x to 18x on moving pages

Same pages, 30 fps, constant quality (crf 28 / crf 36), 1 vCPU:

| Page | JPEG 70 screencast | H.264 crf 28 | H.264 crf 36 |
|---|---|---|---|
| text | 145 kbps (one frame) | 362 kbps | 301 kbps |
| scroll | 27.9 Mbps | 1.6 Mbps | 1.5 Mbps |
| dash | 25.3 Mbps | 3.2 Mbps | 1.4 Mbps |
| video | 6.7 Mbps | 1.3 Mbps | 0.8 Mbps |

Cost: H.264 sends a continuous stream even for a static page (about 300 to 360 kbps), and its encoder uses 50 to 160 % of a CPU (video at 2 vCPU: 160 %). At 0.5 vCPU the encoder held 30 fps at 53 % CPU, with 0.3 to 2.7 Mbps. Whether those 30 fps were delivered or only the encode target is not clear from the log; check before relying on it.

### 3. Flow control works, and is the right lever

Delaying the acknowledgement of each frame throttles the stream smoothly (dash, 1 vCPU): 0 ms 42 fps, 50 ms 31.5 fps, 100 ms 25.5 fps, 200 ms 13.9 fps; bandwidth falls in step (25.1, 18.8, 15.2, 8.3 Mbps). A client on a slow link, or a pane that is not focused, can be slowed down this way without any change on the Chromium side. On the 0.5 vCPU config the CPU is the limit first, so acks of 0 and 50 ms gave the same 18.9 fps.

### 4. Several panes share one Chromium's CPU; they do not add to it

Six panes in one Chromium on the 1 vCPU config: total bandwidth stayed at about 25 Mbps and the frame rate per pane fell from 42.8 to 7.1 fps (1 pane: 42.8, 2: 21.1, 4: 10.6, 6: 7.1). Memory grew only about 24 MB per extra pane (567 MB for one, 686 MB for six). At 0.5 vCPU, six panes got 2.9 fps each. So the number of animated panes per container is a CPU budget question; memory is not the constraint.

### 5. Input is fast enough

Mouse and key events round-trip (event to page effect) in about 15.5 ms at p50 on all configs and 18 to 30 ms at p95. This is the harness's own timing with one pane and a light page; expect it to be worse under load.

### 6. Image quality and format

JPEG 70 at dpr 1: SSIM 0.9984, PSNR 34 dB, 141 kB per full frame. JPEG 85: SSIM 0.9995, 183 kB. WebP 70 through `captureScreenshot`: SSIM 0.9994, 94 kB, but a `captureScreenshot` loop with WebP reached only 9.6 fps against 15.1 fps for JPEG at 1 vCPU, because encoding WebP in Chromium costs CPU (87 % against 56 %). Screencast does not offer WebP. In the browser, decoding a JPEG 70 frame took 5.7 ms, WebP 70 took 10.5 ms (13 ms and 22.6 ms on the 0.5 vCPU run).

### 7. Browser features that work through CDP

All of these worked in all three configs: clipboard both ways (the page must be focused; `Browser.grantPermissions` for the origin), pasting the client's clipboard, copying a selection to the client (the server reads it with `Runtime.evaluate`, the client writes its own clipboard on the person's gesture), IME composition (Chinese input arrived intact), wheel scroll, cursor shape (CDP has no cursor-changed event, so the server reads the computed cursor under the pointer on each move), file upload (client sends the bytes, server writes a temp file and gives CDP the path), download (2 MB in 22 ms to the server; the client is then offered it), a popup or new tab (its own target and its own screencast, 7.6 ms to attach), and JS dialogs (alert, turned into the web UI's own prompt).

### 8. Resize (measured 2026-11-15, after the build's first slice)

The spike's resize cells failed with no first-frame time. The cause was as suspected: a page that does not change sends no new frame. The build's pane handles it by setting the viewport (`Emulation.setDeviceMetricsOverride`) and then stopping and starting the screencast, which delivers a frame of the new size at once.

Measured with the build's probe (`scripts/streamed-browser-probe.mjs`) against a headless srv with the streamed browser on, in a container with 1 vCPU, Chromium 156, no screen, mouse or keyboard, default network with no published port; three runs of 5 panes each, a test page that reports its own size. Time from the resize message to the page's own resize event, and to the first frame whose size is the new one:

| Size | Page saw the resize | First frame of the new size | Missed (of 15 per run) |
|---|---|---|---|
| 800x600 | 8 to 11 ms | 44 to 45 ms | 0 |
| 1024x640 | 3 to 16 ms | 48 to 55 ms | 0 |
| 1600x900 | 3 to 5 ms | 45 to 47 ms | 0 |

Every resize produced a frame of exactly the requested size; no run missed one. A pane that follows its container's size is therefore workable. The same runs, medians per run: first frame after opening a pane 992 to 1,193 ms when Chromium has to start (the first pane), 73 to 123 ms once it runs; a click reaches the page in 4 to 6 ms, a key in 2 to 3 ms, a paste in 5 to 6 ms, a wheel scroll in 79 to 94 ms (not looked into why it is slower than the other input). These are loopback figures: the probe and srv share the container, so they exclude any network and any encryption. A real link is still open.

### 9. Chromium sandbox in the container

Chromium refused to start as root without `--no-sandbox`, and `unshare` of a user namespace is not permitted in the container, so the spike ran with `--no-sandbox`. For a product that runs pages from the open web this needs a decision: a non-root user with a seccomp profile and user namespaces allowed, or accept that the container is the only boundary. `dbus` errors in the log are harmless noise.

## What this means for the build (#18045)

These are my readings of the numbers, to be argued with, not decisions:

1. **Screencast with JPEG is enough to start.** It works with every feature in section 7, a static page is almost free, and a client sees 19 to 42 fps in motion on 1 vCPU. Make the quality adjustable (50 to 85) and default to about 70.
2. **H.264 is the second step, for motion.** It is the only way a moving page fits a normal home connection (1.5 to 3 Mbps against 25 Mbps). It needs an encoder in the container, the client decoder, and a switch: JPEG while the page is still, H.264 when the page moves.
3. **Give each container at least 1 vCPU.** At 0.5 vCPU a single moving pane gets 7 to 18 fps and six panes get under 3 fps each. Cap the animated panes per container, and drive hidden or unfocused panes down with the ack delay (finding 3).
4. **Default to dpr 1** unless the client asks for more; dpr 2 costs about 2.6x the bytes per frame and, on 1 vCPU, a third of the frame rate.
5. **Open before the build:** the sandbox choice (section 9), a longer run than 8 s per cell, and a run with a real remote link between server and client (this spike measured the server side and the browser decode only).

## Limits of this spike

Each cell ran for 8 s, once. One host, one Chromium version (156). The pages are synthetic (`text`, `scroll`, `dash`, `video`), not real sites such as Gmail or a cloud console. No network between server and client, no encryption, no authentication, no cursor or caret overlay. Memory figures are for the Chromium container, not for the process that relays the stream.

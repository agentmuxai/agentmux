#!/usr/bin/env python3
"""HTTPS stand-in for GitHub's release listing and download, for agentmux-install-update.

  GET /api/<tag>            -> www/api/<tag>      (the release listing, GitHub's pretty-printed JSON)
  GET /download/<tag>/<f>   -> 302 /blob/<tag>/<f> (GitHub redirects downloads to asset storage)
  GET /blob/<tag>/<f>       -> www/blob/<tag>/<f>
"""
import http.server, os, ssl, sys

ROOT = "/www"

class H(http.server.SimpleHTTPRequestHandler):
    def translate_path(self, path):
        return os.path.join(ROOT, os.path.normpath(path.split("?", 1)[0]).lstrip("/"))

    def do_GET(self):
        if self.path.startswith("/download/"):
            self.send_response(302)
            self.send_header("Location", "/blob/" + self.path[len("/download/"):])
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        super().do_GET()

    def log_message(self, fmt, *a):
        sys.stdout.write("%s %s\n" % (self.address_string(), fmt % a))
        sys.stdout.flush()

ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.minimum_version = ssl.TLSVersion.TLSv1_2
ctx.load_cert_chain("/certs/server.crt", "/certs/server.key")
srv = http.server.ThreadingHTTPServer(("0.0.0.0", 443), H)
srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
print("mirror up", flush=True)
srv.serve_forever()

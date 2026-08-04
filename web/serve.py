#!/usr/bin/env python3
"""Static file server for the Whisper WASM web harness.

Serves with COOP/COEP headers required for SharedArrayBuffer (future WASM threading).
"""

import http.server
import os
import sys

PORT = 8000

class WASMHandler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header('Cross-Origin-Opener-Policy', 'same-origin')
        self.send_header('Cross-Origin-Embedder-Policy', 'require-corp')
        super().end_headers()

    def guess_type(self, path):
        if path.endswith('.wasm'):
            return 'application/wasm'
        return super().guess_type(path)

if __name__ == '__main__':
    os.chdir(os.path.dirname(os.path.abspath(__file__)))
    print(f'Serving at http://localhost:{PORT}/')
    print(f'Open http://localhost:{PORT}/index.html in Chrome/Edge.')
    http.server.HTTPServer(('0.0.0.0', PORT), WASMHandler).serve_forever()

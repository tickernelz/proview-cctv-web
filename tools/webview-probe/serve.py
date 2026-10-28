import json
import mimetypes
import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

root = Path(sys.argv[1]).resolve()
port = int(sys.argv[2])
inject = os.environ.get("INJECT", "")
devices = [{"id": "d%d" % i, "name": "Camera %d" % i, "online": i % 2 == 0} for i in range(8)]
mimetypes.add_type("application/wasm", ".wasm")
mimetypes.add_type("text/javascript", ".js")
mimetypes.add_type("text/javascript", ".mjs")


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def send(self, body, kind):
        self.send_response(200)
        self.send_header("content-type", kind)
        self.send_header("cache-control", "no-store")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        self.send(b"ok", "text/plain")
        sys.stdout.write(body.decode() + "\n")
        sys.stdout.flush()
        threading.Thread(target=server.shutdown).start()

    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/api/devices":
            return self.send(json.dumps(devices).encode(), "application/json")
        file = (root / path.lstrip("/")).resolve()
        if file.is_dir():
            file = file / "index.html"
        if not file.is_file() or root not in file.parents:
            file = root / "index.html"
        body = file.read_bytes()
        if file.suffix == ".html" and inject:
            text = body.decode()
            at = text.find("<head")
            close = text.find(">", at) + 1 if at >= 0 else 0
            body = (text[:close] + inject + text[close:]).encode()
        self.send(body, mimetypes.guess_type(file.name)[0] or "application/octet-stream")


server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
timer = threading.Timer(60, server.shutdown)
timer.daemon = True
timer.start()
server.serve_forever()

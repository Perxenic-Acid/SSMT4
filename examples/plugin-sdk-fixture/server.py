from __future__ import annotations

import argparse
import ctypes
import json
import mimetypes
import os
import secrets
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlsplit

HERE = Path(__file__).resolve().parent
UI_ROOT = HERE / "ui"
TOKEN = ""
DATA_ROOT = HERE
BRIDGE_STATUS: dict[str, object] = {"ok": False, "reason": "bridge not called"}


def parent_alive(pid: int) -> bool:
    if not pid:
        return True
    kernel32 = ctypes.windll.kernel32
    handle = kernel32.OpenProcess(0x1000, False, pid)
    if not handle:
        return False
    try:
        code = ctypes.c_ulong()
        return bool(kernel32.GetExitCodeProcess(handle, ctypes.byref(code))) and code.value == 259
    finally:
        kernel32.CloseHandle(handle)


def watch_parent(pid: int) -> None:
    while True:
        time.sleep(2)
        if not parent_alive(pid):
            os._exit(0)


class Handler(BaseHTTPRequestHandler):
    server_version = "SSMTPluginSDKFixture/1.0"

    def reply(self, status: int, body: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        route = urlsplit(self.path)
        if route.path.startswith("/ui/"):
            relative = unquote(route.path[len("/ui/"):]) or "index.html"
            target = (UI_ROOT / relative).resolve()
            if not target.is_relative_to(UI_ROOT.resolve()) or not target.is_file():
                return self.reply(404, b"not found", "text/plain")
            return self.reply(
                200,
                target.read_bytes(),
                mimetypes.guess_type(target.name)[0] or "application/octet-stream",
            )
        if self.headers.get("X-Editor-Token") != TOKEN:
            return self.reply(403, b"invalid token", "text/plain")
        if route.path == "/api/health":
            return self.reply(
                200,
                json.dumps({
                    "service": "ssmt-plugin-sdk-fixture",
                    "protocolVersion": 1,
                    "instanceToken": TOKEN,
                    "pid": os.getpid(),
                }).encode(),
                "application/json",
            )
        if route.path == "/api/context":
            return self.reply(
                200,
                json.dumps({
                    "service": "ssmt-plugin-sdk-fixture",
                    "dataRoot": str(DATA_ROOT),
                }).encode(),
                "application/json",
            )
        if route.path == "/api/bridge-status":
            return self.reply(
                200,
                json.dumps(BRIDGE_STATUS).encode(),
                "application/json",
            )
        return self.reply(404, b"not found", "text/plain")

    def do_POST(self) -> None:
        route = urlsplit(self.path)
        if self.headers.get("X-Editor-Token") != TOKEN:
            return self.reply(403, b"invalid token", "text/plain")
        if route.path != "/api/bridge-result":
            return self.reply(404, b"not found", "text/plain")
        length = int(self.headers.get("Content-Length", "0"))
        if length <= 0 or length > 8192:
            return self.reply(400, b"invalid body length", "text/plain")
        try:
            payload = json.loads(self.rfile.read(length))
        except (json.JSONDecodeError, UnicodeDecodeError):
            return self.reply(400, b"invalid json", "text/plain")
        if not isinstance(payload, dict):
            return self.reply(400, b"invalid payload", "text/plain")
        global BRIDGE_STATUS
        BRIDGE_STATUS = payload
        return self.reply(200, b"{}", "application/json")


def main() -> None:
    global TOKEN, DATA_ROOT
    parser = argparse.ArgumentParser()
    parser.add_argument("--mods", required=True)
    parser.add_argument("--data", required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--instance-token", required=True)
    parser.add_argument("--parent-pid", type=int, default=0)
    args = parser.parse_args()
    TOKEN = args.instance_token
    DATA_ROOT = Path(args.data)
    DATA_ROOT.mkdir(parents=True, exist_ok=True)
    (DATA_ROOT / "mods.txt").write_text(args.mods, encoding="utf-8")
    if args.parent_pid:
        threading.Thread(target=watch_parent, args=(args.parent_pid,), daemon=True).start()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"SSMT plugin SDK fixture: http://127.0.0.1:{args.port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()

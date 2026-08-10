#!/usr/bin/env python3
"""Minimal deterministic Base JSON-RPC fixture for the launcher smoke test."""

import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port-file", required=True, type=Path)
    parser.add_argument("--wallet", required=True)
    args = parser.parse_args()
    wallet_word = "0x" + ("0" * 24) + args.wallet.removeprefix("0x").lower()
    zero_word = "0x" + ("0" * 64)

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length", "0"))
            request = json.loads(self.rfile.read(length))
            method = request.get("method")
            if method == "eth_chainId":
                result = "0x2105"
            elif method == "eth_call":
                call = (request.get("params") or [{}])[0]
                destination = str(call.get("to", "")).lower()
                result = wallet_word if destination == "0x0000000000000000000000000000000000000001" else zero_word
            else:
                self.send_error(400, "unsupported test RPC method")
                return
            body = json.dumps({"jsonrpc": "2.0", "id": request.get("id"), "result": result}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format: str, *args: object) -> None:
            del format, args
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    args.port_file.write_text(str(server.server_port), encoding="ascii")
    server.serve_forever()


if __name__ == "__main__":
    main()

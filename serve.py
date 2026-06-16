#!/usr/bin/env python3
"""Serve web/ over HTTPS (phones require a secure context for mic access).

Usage: ./serve.py [--http] [port]
Generates a self-signed cert into .scratch/ on first run; accept the
browser warning on the phone once. --http serves plain HTTP for
localhost-only desktop testing.
"""
import http.server
import os
import socket
import ssl
import subprocess
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
WEB = os.path.join(ROOT, "web")
SCRATCH = os.path.join(ROOT, ".scratch")
CERT = os.path.join(SCRATCH, "cert.pem")
KEY = os.path.join(SCRATCH, "key.pem")


def lan_ip():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("8.8.8.8", 80))
        return s.getsockname()[0]
    except OSError:
        return "127.0.0.1"
    finally:
        s.close()


def ensure_cert(ip):
    if os.path.exists(CERT) and os.path.exists(KEY):
        return
    os.makedirs(SCRATCH, exist_ok=True)
    subprocess.run(
        [
            "openssl", "req", "-x509", "-newkey", "rsa:2048",
            "-keyout", KEY, "-out", CERT, "-days", "825", "-nodes",
            "-subj", "/CN=tunerfish",
            "-addext", f"subjectAltName=IP:{ip},DNS:localhost",
        ],
        check=True,
    )


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
    }

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=WEB, **kwargs)

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def main():
    use_tls = "--http" not in sys.argv
    ports = [int(a) for a in sys.argv[1:] if a.isdigit()]
    port = ports[0] if ports else (8443 if use_tls else 8000)
    ip = lan_ip()
    server = http.server.ThreadingHTTPServer(("0.0.0.0", port), Handler)
    if use_tls:
        ensure_cert(ip)
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(CERT, KEY)
        server.socket = ctx.wrap_socket(server.socket, server_side=True)
        print(f"phone:   https://{ip}:{port}/")
        print(f"desktop: https://localhost:{port}/")
    else:
        print(f"desktop: http://localhost:{port}/")
    server.serve_forever()


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
import os
import socket
import sys

runtime = os.environ.get("XDG_RUNTIME_DIR", "/tmp")
sock_path = os.path.join(runtime, "panora.sock")
frame = b'{"method":"Status","params":{' + (b"x" * 70000) + b"}}\n"

sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.settimeout(3)
sock.connect(sock_path)
sock.sendall(frame)
reply = sock.recv(4096).decode("utf-8", "replace")
sock.close()
print(reply, end="")
if "too large" not in reply.lower() and "limit" not in reply.lower() and "error" not in reply.lower():
    sys.exit(1)

"""Ephemeral-port allocators shared by SRT/RTP/TCP loopback tests.

Ask the OS for a free port, then release it immediately. The kernel
allocates UDP and TCP ports from separate namespaces, so either
family's ephemeral integer suffices for any loopback test regardless
of which protocol it targets. The tiny TOCTOU window between release
and rebind is fine since pytest runs these tests sequentially by
default.
"""

from __future__ import annotations

import socket


def free_tcp_port() -> int:
    """Ask the OS for an ephemeral TCP port, then release it."""
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def free_udp_port() -> int:
    """Ask the OS for an ephemeral UDP port, then release it."""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port

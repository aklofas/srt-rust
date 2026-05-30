//! Port / socket allocation for network tests.
//!
//! Test-binary consolidation (the 2026-05-28 overhaul) folded many loopback
//! tests into a handful of per-domain binaries. Cargo runs binaries
//! sequentially, but libtest parallelises *within* a binary, so consolidated
//! network tests now contend on `127.0.0.1` ports far more than before — the
//! documented flake class in `feedback_test_binary_consolidation_concurrency.md`.
//!
//! Two tools here reduce that contention:
//!
//!   * [`reserve_tcp_listener`] / [`reserve_udp_socket`] bind an ephemeral
//!     (`:0`) port and hand the *already-bound* socket to the code under test.
//!     There is no bind→drop→reuse window for another test to race into, so
//!     prefer these wherever an API can accept a pre-bound socket.
//!
//!   * [`claim_fixed_port`] records fixed ports in a process-local registry and
//!     panics if two tests in the same binary claim the same one — turning a
//!     nondeterministic `EADDRINUSE` flake into a deterministic, named failure.
//!
//! `reserve_*` returns the bound socket itself rather than a bare port number
//! on purpose: reading a port from a `:0` bind and then dropping the socket
//! reintroduces the very race these helpers exist to remove.

use std::collections::HashMap;
use std::net::{TcpListener, UdpSocket};
use std::sync::{Mutex, OnceLock};

/// Bind a TCP listener on an OS-assigned ephemeral port on `127.0.0.1`.
///
/// Pass the returned listener to the code under test, or read
/// `listener.local_addr()?.port()` for an API that needs a port number — but
/// keep the listener alive until the port is actually in use.
pub fn reserve_tcp_listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").expect("bind ephemeral TCP port on 127.0.0.1")
}

/// Bind a UDP socket on an OS-assigned ephemeral port on `127.0.0.1`.
pub fn reserve_udp_socket() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0").expect("bind ephemeral UDP socket on 127.0.0.1")
}

fn fixed_port_registry() -> &'static Mutex<HashMap<u16, String>> {
    static REGISTRY: OnceLock<Mutex<HashMap<u16, String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record a hard-coded port as in use by this process, panicking if another
/// caller already claimed it.
///
/// Ephemeral ports are always preferable; use this only when a test must use a
/// fixed port (e.g. a URL baked into a C example). The `reason` is shown in the
/// panic message of whichever claim loses the race, so make it identify the
/// test.
pub fn claim_fixed_port(port: u16, reason: &str) {
    // Read/insert under the lock, then DROP the guard before any panic. A
    // double-claim must not panic while holding the mutex: that would poison it
    // and turn an unrelated parallel claim into a spurious PoisonError — exactly
    // the cross-test flake this module exists to remove. Tolerate a poisoned
    // lock too (a panic elsewhere shouldn't wedge the registry).
    let already_held = {
        let mut registry = fixed_port_registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match registry.get(&port) {
            Some(existing) => Some(existing.clone()),
            None => {
                registry.insert(port, reason.to_string());
                None
            }
        }
    };
    if let Some(existing) = already_held {
        panic!(
            "fixed port {port} double-claimed in this process: already held by \
             {existing:?}, now also requested by {reason:?} — give one of them an \
             ephemeral port via reserve_tcp_listener/reserve_udp_socket, or a \
             distinct fixed port"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_tcp_listener_has_a_real_port() {
        let listener = reserve_tcp_listener();
        let port = listener.local_addr().unwrap().port();
        assert_ne!(port, 0, "ephemeral bind must resolve to a concrete port");
    }

    #[test]
    fn two_tcp_reservations_get_distinct_ports() {
        let a = reserve_tcp_listener();
        let b = reserve_tcp_listener();
        assert_ne!(
            a.local_addr().unwrap().port(),
            b.local_addr().unwrap().port(),
            "concurrently-held ephemeral binds must not alias"
        );
    }

    #[test]
    fn reserved_udp_socket_has_a_real_port() {
        let socket = reserve_udp_socket();
        assert_ne!(socket.local_addr().unwrap().port(), 0);
    }

    #[test]
    fn fixed_port_claimed_once_is_ok() {
        // Distinct port per test so the process-local registry can't collide
        // with the duplicate-claim test running in parallel in this binary.
        claim_fixed_port(51001, "fixed_port_claimed_once_is_ok");
    }

    #[test]
    #[should_panic(expected = "double-claimed")]
    fn fixed_port_double_claim_panics() {
        claim_fixed_port(51002, "fixed_port_double_claim_panics/first");
        claim_fixed_port(51002, "fixed_port_double_claim_panics/second");
    }
}

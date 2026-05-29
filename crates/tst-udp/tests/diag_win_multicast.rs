//! TEMPORARY windows-only diagnostic — find the multicast loopback recipe.
//!
//! We can't run Windows locally, so this probe runs on the windows-msvc CI
//! runner (non-gating) to answer the open questions blocking multicast
//! un-gating (see `project_windows_multicast_investigation`):
//!
//!   1. What is the runner's real multicast-capable NIC IPv4? (loopback
//!      `127.0.0.1` is NOT multicast-capable on Windows.)
//!   2. Does Winsock accept `IP_MULTICAST_IF = 127.0.0.1`, or reject it?
//!   3. Which (send iface / recv-join iface / IP_MULTICAST_LOOP side)
//!      combination actually delivers a same-process loopback round-trip on
//!      Windows — where `IP_MULTICAST_LOOP` is RECEIVER-side (opposite of BSD)?
//!
//! The test never asserts delivery (it's a probe, not a gate) — it prints a
//! result table. Read it via the dedicated `--no-capture` CI step. DELETE this
//! file + that CI step once the recipe is known and the real round-trip tests
//! are un-gated.
#![cfg(target_os = "windows")]

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::Duration;

use socket2::SockRef;

/// Discover the runner's primary (route-to-internet) IPv4 — the real NIC a
/// Windows multicast send must egress on. No packet is sent: UDP `connect`
/// only fixes the kernel's source-address selection so `local_addr` reports
/// the chosen interface IP.
fn primary_nic_v4() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    match s.local_addr().ok()? {
        SocketAddr::V4(a) => Some(*a.ip()),
        SocketAddr::V6(_) => None,
    }
}

/// One probe config. `send_iface`/`join_iface` are the IP_MULTICAST_IF and
/// join-interface addresses; `loop_send`/`loop_recv` toggle IP_MULTICAST_LOOP
/// on each socket.
struct Probe {
    label: &'static str,
    group: Ipv4Addr,
    port: u16,
    send_iface: Option<Ipv4Addr>,
    join_iface: Ipv4Addr,
    loop_send: bool,
    loop_recv: bool,
}

/// Attempt a same-process multicast round-trip. Returns Ok(true) if the
/// payload was delivered within the recv timeout, Ok(false) if not delivered,
/// Err(step) naming the first socket call that failed (e.g. a Winsock reject
/// of IP_MULTICAST_IF=loopback).
fn run_probe(p: &Probe) -> Result<bool, String> {
    let group = p.group;
    let bind_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, p.port));

    let recv = UdpSocket::bind(bind_addr).map_err(|e| format!("recv bind: {e}"))?;
    recv.join_multicast_v4(&group, &p.join_iface).map_err(|e| {
        format!(
            "join_multicast_v4(group={group}, iface={}): {e}",
            p.join_iface
        )
    })?;
    recv.set_multicast_loop_v4(p.loop_recv)
        .map_err(|e| format!("recv set_multicast_loop_v4({}): {e}", p.loop_recv))?;
    recv.set_read_timeout(Some(Duration::from_millis(1200)))
        .map_err(|e| format!("recv set_read_timeout: {e}"))?;

    let send = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("send bind: {e}"))?;
    if let Some(if4) = p.send_iface {
        // std::net has no set_multicast_if_v4; socket2's SockRef borrows the
        // OS socket without taking ownership and exposes the cross-platform
        // setsockopt (this is exactly what the production fix will call).
        SockRef::from(&send)
            .set_multicast_if_v4(&if4)
            .map_err(|e| format!("set_multicast_if_v4({if4}): {e}"))?;
    }
    send.set_multicast_loop_v4(p.loop_send)
        .map_err(|e| format!("send set_multicast_loop_v4({}): {e}", p.loop_send))?;
    send.set_multicast_ttl_v4(1)
        .map_err(|e| format!("send set_multicast_ttl_v4: {e}"))?;

    let payload = [0x47u8; 32];
    let dst = SocketAddr::V4(SocketAddrV4::new(group, p.port));
    send.send_to(&payload, dst)
        .map_err(|e| format!("send_to({dst}): {e}"))?;

    let mut buf = [0u8; 64];
    match recv.recv_from(&mut buf) {
        Ok((n, _)) => Ok(n >= payload.len() && buf[..payload.len()] == payload),
        Err(_) => Ok(false), // timeout / WouldBlock => not delivered
    }
}

#[test]
fn diag_win_multicast_recipe() {
    let nic = primary_nic_v4();
    let mut report = String::new();
    report.push_str("\n========== DIAG_WIN_MULTICAST ==========\n");
    report.push_str(&format!("primary NIC v4: {nic:?}\n"));

    // Direct answer to "does Winsock accept IP_MULTICAST_IF = 127.0.0.1?"
    {
        let s = UdpSocket::bind("0.0.0.0:0").unwrap();
        let res = SockRef::from(&s).set_multicast_if_v4(&Ipv4Addr::LOCALHOST);
        report.push_str(&format!("set_multicast_if_v4(127.0.0.1) -> {res:?}\n"));
    }
    if let Some(nic) = nic {
        let s = UdpSocket::bind("0.0.0.0:0").unwrap();
        let res = SockRef::from(&s).set_multicast_if_v4(&nic);
        report.push_str(&format!("set_multicast_if_v4(NIC {nic}) -> {res:?}\n"));
    }

    let group: Ipv4Addr = Ipv4Addr::new(239, 201, 7, 7);
    let unspec = Ipv4Addr::UNSPECIFIED;
    let loop4 = Ipv4Addr::LOCALHOST;
    // Distinct port per probe so probes never cross-deliver.
    let mut probes: Vec<Probe> = Vec::new();
    if let Some(nic) = nic {
        probes.push(Probe {
            label: "NIC iface, loop_recv",
            group,
            port: 56120,
            send_iface: Some(nic),
            join_iface: nic,
            loop_send: false,
            loop_recv: true,
        });
        probes.push(Probe {
            label: "NIC iface, loop_send",
            group,
            port: 56122,
            send_iface: Some(nic),
            join_iface: nic,
            loop_send: true,
            loop_recv: false,
        });
        probes.push(Probe {
            label: "NIC iface, loop both",
            group,
            port: 56124,
            send_iface: Some(nic),
            join_iface: nic,
            loop_send: true,
            loop_recv: true,
        });
        probes.push(Probe {
            label: "NIC send, join 0.0.0.0, loop both",
            group,
            port: 56126,
            send_iface: Some(nic),
            join_iface: unspec,
            loop_send: true,
            loop_recv: true,
        });
        probes.push(Probe {
            label: "default iface (0.0.0.0), loop both",
            group,
            port: 56128,
            send_iface: None,
            join_iface: unspec,
            loop_send: true,
            loop_recv: true,
        });
    }
    probes.push(Probe {
        label: "loopback iface 127.0.0.1, loop both",
        group,
        port: 56130,
        send_iface: Some(loop4),
        join_iface: loop4,
        loop_send: true,
        loop_recv: true,
    });

    for p in &probes {
        let outcome = match run_probe(p) {
            Ok(true) => "DELIVERED".to_string(),
            Ok(false) => "no-delivery".to_string(),
            Err(step) => format!("ERR @ {step}"),
        };
        report.push_str(&format!("  [{outcome:>16}]  {}\n", p.label));
    }
    report.push_str("========================================\n");

    // Print to both streams so it shows under --no-capture regardless of which
    // the CI log surfaces.
    eprint!("{report}");
    print!("{report}");
}

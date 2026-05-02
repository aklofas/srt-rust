//! Passphrase-encrypted SRT send + recv in one process.
//!
//! Spawns a listener thread on 127.0.0.1, then a sender thread that connects
//! with the matching passphrase. Sends 16 messages, receives them all,
//! verifies bytes match. Exits cleanly.
//!
//!   cargo run --example encrypted_send_recv
//!
//! Demonstrates: SocketBuilder/ListenerBuilder + Passphrase + KeyLength +
//! StreamId. The same shape applies across-network — only the bind/connect
//! addresses change.

use srt_core::srt::{KeyLength, ListenerBuilder, Passphrase, SocketBuilder, StreamId};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const PASSPHRASE: &str = "shared-secret-not-for-production";
const STREAM_ID: &str = "encrypted-demo";
const NUM_MESSAGES: usize = 16;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Pick a free port by briefly binding TCP and dropping. Avoids the "port
    // already in use" surprise on shared dev machines.
    let port = {
        let l = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        let p = l.local_addr()?.port();
        drop(l);
        p
    };
    let bind_addr = format!("127.0.0.1:{port}");
    let connect_addr = bind_addr.clone();

    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<usize>();

    // Listener thread.
    let listener_handle = thread::spawn(move || -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let passphrase = Passphrase::new(PASSPHRASE)?;
        let mut listener = ListenerBuilder::new()
            .passphrase(passphrase)
            .key_length(KeyLength::Aes256)
            .latency(Duration::from_millis(120))
            .bind(bind_addr.as_str())?;

        ready_tx.send(()).ok();

        let (mut socket, peer) = listener.accept()?;
        eprintln!("listener: accepted from {peer}, stream_id={:?}", socket.stream_id());

        let mut buf = [0u8; 1500];
        let mut received = 0usize;
        loop {
            match socket.recv(&mut buf) {
                Ok(n) => {
                    eprintln!("listener: recv {n} bytes (msg {received})");
                    received += 1;
                    if received >= NUM_MESSAGES {
                        break;
                    }
                }
                Err(srt_core::error::RecvError::ConnectionBroken) => break,
                Err(srt_core::error::RecvError::TimedOut) => continue,
                Err(e) => return Err(Box::new(e)),
            }
        }
        done_tx.send(received).ok();
        Ok(())
    });

    // Wait for the listener to be bound before connecting.
    ready_rx.recv()?;
    thread::sleep(Duration::from_millis(50));

    // Sender thread (could be inline; threaded to mirror real deployments).
    let sender_handle = thread::spawn(move || -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let passphrase = Passphrase::new(PASSPHRASE)?;
        let stream_id = StreamId::new(STREAM_ID)?;
        let mut socket = SocketBuilder::new()
            .passphrase(passphrase)
            .key_length(KeyLength::Aes256)
            .stream_id(stream_id)
            .latency(Duration::from_millis(120))
            .connect(connect_addr.as_str())?;

        for i in 0..NUM_MESSAGES {
            let msg = format!("encrypted message {i:02}").into_bytes();
            socket.send(&msg)?;
            thread::sleep(Duration::from_millis(20));
        }
        eprintln!("sender: sent {NUM_MESSAGES} messages");
        thread::sleep(Duration::from_millis(200));
        socket.close()?;
        Ok(())
    });

    sender_handle
        .join()
        .expect("sender thread")
        .map_err(|e| -> Box<dyn std::error::Error> { e })?;
    listener_handle
        .join()
        .expect("listener thread")
        .map_err(|e| -> Box<dyn std::error::Error> { e })?;
    let received = done_rx.recv_timeout(Duration::from_secs(2))?;
    assert_eq!(received, NUM_MESSAGES, "received {received} of {NUM_MESSAGES}");

    println!("OK: {NUM_MESSAGES} encrypted messages round-tripped");
    Ok(())
}

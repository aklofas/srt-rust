//! Behavior suite for `ReconnectMode::Background`.
//!
//! All synchronization is deadline-polling (`wait_until`) — never bare
//! sleeps-as-sync. Timing assertions carry >=5x margins for slow CI.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tst_core::transport::{Transport, TransportError};
use tst_pipeline::{
    BackoffStrategy, ManagedTransport, OverflowPolicy, ReconnectMode, ReconnectPolicy,
};

/// What the next inner send should do. Empty script => Ok.
#[derive(Clone, Copy, Debug)]
enum SendOutcome {
    Ok,
    #[allow(dead_code)] // constructed starting in a later task of this arc
    Backpressure,
    Broken,
}

/// Script-driven mock transport. Successful sends land in a log SHARED
/// with the test rig (survives factory rebuilds, so FIFO can be asserted
/// across reconnect cycles).
struct ScriptedTransport {
    script: Arc<Mutex<VecDeque<SendOutcome>>>,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
    max_payload: usize,
    alive: bool,
}

impl Transport for ScriptedTransport {
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError> {
        if msg.len() > self.max_payload {
            return Err(TransportError::TooLarge {
                len: msg.len(),
                max: self.max_payload,
            });
        }
        if !self.alive {
            return Err(TransportError::Broken {
                msg: "dead".into(),
                errno_code: None,
            });
        }
        match self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(SendOutcome::Ok)
        {
            SendOutcome::Ok => {
                self.sent.lock().unwrap().push(msg.to_vec());
                Ok(())
            }
            SendOutcome::Backpressure => Err(TransportError::Backpressure {
                msg: "bp".into(),
                errno_code: None,
            }),
            SendOutcome::Broken => {
                self.alive = false;
                Err(TransportError::Broken {
                    msg: "scripted break".into(),
                    errno_code: None,
                })
            }
        }
    }
    fn max_payload(&self) -> usize {
        self.max_payload
    }
    fn is_alive(&self) -> bool {
        self.alive
    }
    fn close(&mut self) {
        self.alive = false;
    }
}

/// Shared state backing every transport the rig (or its factories) builds.
struct Rig {
    script: Arc<Mutex<VecDeque<SendOutcome>>>,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
    factory_calls: Arc<AtomicU32>,
}

impl Rig {
    fn new() -> Self {
        Self {
            script: Arc::new(Mutex::new(VecDeque::new())),
            sent: Arc::new(Mutex::new(Vec::new())),
            factory_calls: Arc::new(AtomicU32::new(0)),
        }
    }
    fn transport(&self) -> ScriptedTransport {
        self.transport_with_payload(1316)
    }
    fn transport_with_payload(&self, max_payload: usize) -> ScriptedTransport {
        ScriptedTransport {
            script: Arc::clone(&self.script),
            sent: Arc::clone(&self.sent),
            max_payload,
            alive: true,
        }
    }
    fn factory(
        &self,
        fail_first: u32,
    ) -> impl Fn() -> Result<ScriptedTransport, TransportError> + Send + Sync + 'static {
        self.factory_with_payload(fail_first, 1316)
    }
    fn factory_with_payload(
        &self,
        fail_first: u32,
        max_payload: usize,
    ) -> impl Fn() -> Result<ScriptedTransport, TransportError> + Send + Sync + 'static {
        let calls = Arc::clone(&self.factory_calls);
        let script = Arc::clone(&self.script);
        let sent = Arc::clone(&self.sent);
        move || {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            if n < fail_first {
                Err(TransportError::Broken {
                    msg: "factory down".into(),
                    errno_code: None,
                })
            } else {
                Ok(ScriptedTransport {
                    script: Arc::clone(&script),
                    sent: Arc::clone(&sent),
                    max_payload,
                    alive: true,
                })
            }
        }
    }
    fn push_outcome(&self, o: SendOutcome) {
        self.script.lock().unwrap().push_back(o);
    }
    fn sent_snapshot(&self) -> Vec<Vec<u8>> {
        self.sent.lock().unwrap().clone()
    }
}

fn bg_policy(max_attempts: Option<u32>, backoff: BackoffStrategy) -> ReconnectPolicy {
    ReconnectPolicy {
        max_attempts,
        backoff,
        gap_buffer_capacity: 64,
        overflow_policy: OverflowPolicy::DropOldest,
        mode: ReconnectMode::Background,
    }
}

/// Poll until `cond` or panic after `deadline`.
fn wait_until(deadline: Duration, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(
            start.elapsed() < deadline,
            "condition not met within {deadline:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn background_send_never_blocks_during_outage() {
    let rig = Rig::new();
    rig.push_outcome(SendOutcome::Broken); // first send breaks the inner
    // Factory never succeeds; backoff parks the worker 5s per attempt.
    // Every send during the outage must return immediately regardless.
    let policy = bg_policy(None, BackoffStrategy::Constant(Duration::from_secs(5)));
    let mut managed = ManagedTransport::new(rig.transport(), rig.factory(u32::MAX), policy);
    for i in 0..20u8 {
        let t0 = Instant::now();
        managed
            .send_bytes(&[i])
            .expect("background mode accepts into the gap buffer");
        assert!(
            t0.elapsed() < Duration::from_secs(1),
            "send_bytes blocked on reconnect backoff: {:?}",
            t0.elapsed()
        );
    }
    // Drop detaches; the worker wakes from its 5s wait on the signal and exits.
}

#[test]
fn background_reconnect_drains_fifo_then_resumes_direct() {
    let rig = Rig::new();
    rig.push_outcome(SendOutcome::Ok); //     msg 0: direct send
    rig.push_outcome(SendOutcome::Broken); // msg 1: breaks the inner (msg 1 must be queued, not lost)
    let policy = bg_policy(None, BackoffStrategy::Constant(Duration::from_millis(20)));
    let mut managed = ManagedTransport::new(rig.transport(), rig.factory(2), policy);

    managed.send_bytes(&[0]).unwrap();
    managed.send_bytes(&[1]).unwrap(); // break -> enqueued, worker spawned
    managed.send_bytes(&[2]).unwrap(); // gate -> gap
    managed.send_bytes(&[3]).unwrap(); // gate -> gap
    // Factory fails twice; 3rd call succeeds; worker drains 1,2,3 in order.
    wait_until(Duration::from_secs(10), || rig.sent_snapshot().len() == 4);
    assert_eq!(
        rig.sent_snapshot(),
        vec![vec![0], vec![1], vec![2], vec![3]],
        "exact FIFO across the outage — no loss, no reorder, no double-send"
    );
    // After drain the worker exits; the next send may go direct or ride a
    // benign gate window — either way it lands, in order.
    managed.send_bytes(&[4]).unwrap();
    wait_until(Duration::from_secs(10), || rig.sent_snapshot().len() == 5);
    assert_eq!(rig.sent_snapshot()[4], vec![4]);
}

#[test]
fn background_dropoldest_evicts_and_counts_deterministically() {
    let rig = Rig::new();
    rig.push_outcome(SendOutcome::Broken);
    // 200ms backoff >> the microseconds the 6 sends below take, so the
    // eviction sequence is deterministic (no concurrent drain).
    let mut policy = bg_policy(None, BackoffStrategy::Constant(Duration::from_millis(200)));
    policy.gap_buffer_capacity = 2;
    let mut managed = ManagedTransport::new(rig.transport(), rig.factory(3), policy);
    let stats = managed.stats_handle();

    for i in 0..6u8 {
        managed.send_bytes(&[i]).unwrap();
    }
    // Queue evolution (cap 2): [0] [0,1] [1,2] [2,3] [3,4] [4,5] => 4 evicted.
    wait_until(Duration::from_secs(15), || {
        stats.stats().unwrap().reconnect_successes == 1
    });
    wait_until(Duration::from_secs(15), || {
        !stats.stats().unwrap().reconnecting
    });
    let s = stats.stats().unwrap();
    assert_eq!(s.gap_messages_dropped, 4);
    assert_eq!(s.reconnect_attempts, 4, "3 factory failures + 1 success");
    assert_eq!(
        rig.sent_snapshot(),
        vec![vec![4], vec![5]],
        "only the surviving tail is delivered, in order"
    );
    assert_eq!(s.gap_len, 0);
}

/// Fix round 1 (review finding I1): a worker that unwinds — e.g. a
/// user-supplied `factory()` that panics on `unwrap()` during DNS/socket
/// setup — must not leave `bg_active` stuck true. Pre-fix that would wedge
/// every future send into the enqueue-and-return-Ok branch forever, with
/// no replacement worker ever able to spawn: unbounded silent loss
/// reported as healthy. The panic below is expected to print to captured
/// test output.
#[test]
fn background_worker_panic_recovers_via_abnormal_give_up() {
    let rig = Rig::new();
    rig.push_outcome(SendOutcome::Broken); // msg 0's send breaks the inner -> worker spawns
    let policy = bg_policy(None, BackoffStrategy::Constant(Duration::from_millis(10)));

    // Custom factory (not rig.factory(), which never panics): first call
    // panics — simulating e.g. an `unwrap()` on DNS/socket setup — every
    // later call succeeds with a normal working transport.
    let calls = Arc::clone(&rig.factory_calls);
    let script = Arc::clone(&rig.script);
    let sent = Arc::clone(&rig.sent);
    let factory = move || -> Result<ScriptedTransport, TransportError> {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            panic!("test factory panic");
        }
        Ok(ScriptedTransport {
            script: Arc::clone(&script),
            sent: Arc::clone(&sent),
            max_payload: 1316,
            alive: true,
        })
    };

    let mut managed = ManagedTransport::new(rig.transport(), factory, policy);
    let stats = managed.stats_handle();

    managed.send_bytes(&[0]).unwrap(); // breaks the inner; enqueued; worker spawns and panics
    wait_until(Duration::from_secs(10), || {
        !stats.stats().unwrap().reconnecting
    });

    // Report-once: the next send surfaces the abnormal give-up, and its
    // own bytes are NOT queued (same contract as a normal give-up).
    match managed.send_bytes(&[1]).unwrap_err() {
        TransportError::Broken { msg, .. } => {
            assert!(
                msg.contains("aborted"),
                "expected an abnormal give-up message, got: {msg}"
            );
        }
        other => panic!("expected Broken, got {other:?}"),
    }

    // The give-up is consumed — this send starts a fresh cycle instead of
    // repeating the abnormal error, and the factory's 2nd call succeeds.
    managed.send_bytes(&[2]).unwrap();
    wait_until(Duration::from_secs(10), || rig.sent_snapshot().len() == 2);
    assert_eq!(
        rig.sent_snapshot(),
        vec![vec![0], vec![2]],
        "msg 0 (queued through the crash) then msg 2, in order; msg 1 was never queued"
    );
    assert!(stats.stats().unwrap().reconnect_successes >= 1);
}

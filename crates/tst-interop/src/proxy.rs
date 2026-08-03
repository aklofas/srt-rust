//! `proxy` subcommand: a UDP impairment relay driven by [`crate::impair::
//! Engine`]'s deterministic per-packet decisions.
//!
//! # Topology and direction semantics
//!
//! A single UDP socket is bound at `listen`. Two kinds of peer talk to
//! it:
//!
//! - The **client** (whoever a test/CLI user points at this proxy in
//!   place of the real destination — e.g. an SRT caller). Its address is
//!   unknown up front and is *learned* from the first datagram this
//!   socket receives whose source isn't `forward` (see below), and can
//!   be RE-learned from a different source later — but only once the
//!   PREVIOUS client peer has gone quiet for at least
//!   `CLIENT_RELEARN_GRACE` (that constant's own doc comment has the
//!   full "why": a bare "first datagram wins forever" rule is
//!   incompatible with a client that legitimately reconnects from a
//!   fresh source port, e.g. `send --managed`'s reconnect factory).
//!   Every datagram from the current client peer is the one
//!   [`crate::impair::Engine`] governs: `Engine::decide` runs, and the
//!   result (drop / forward / dup-forward, each with its own delay)
//!   determines what — if anything — gets relayed on to `forward`, and
//!   when.
//! - The **real destination** at `forward` (e.g. the actual SRT
//!   listener). Datagrams whose source address equals `forward` are
//!   replies flowing back toward the client (SRT ACKs, RTCP, etc.) —
//!   relayed to the learned client peer **immediately, with no
//!   impairment**. SRT (and any other reliable-transport handshake)
//!   needs this reverse path to actually flow for the forward
//!   direction's impairment to mean anything; impairing the ACK path
//!   too is a config knob for another day (YAGNI — nothing in this
//!   arc's evidence goals asks for it).
//!
//! Using one socket for both roles (rather than a second socket dialed
//! at `forward`) is what makes "reply from `forward`" a simple source-
//! address comparison, and it's also *why* the real destination sees the
//! relayed client traffic as if it originated from this proxy's `listen`
//! address — which is exactly what makes its replies come back to this
//! same socket in the first place.
//!
//! # Delayed forwarding
//!
//! Every decision that doesn't `Drop` is queued into a `BinaryHeap`
//! ordered by `(due_instant, seq)` — `seq` (a monotonically increasing
//! counter) breaks ties between same-instant entries so heap drain order
//! is deterministic even when two decisions compute the same `delay_ms`
//! from the same `Instant::now()` call. The main loop drains every entry
//! whose `due_instant` has passed after each `recv_from` attempt
//! (whether or not that attempt actually received anything), and — on
//! the way out of [`run`] — flushes every remaining entry immediately
//! regardless of its due time, rather than silently discarding packets
//! that were already decided (and counted) as forwarded.
//!
//! # `reordered`'s definition
//!
//! [`Engine::decide`]'s `Action::Forward`/`Action::DupForward` carry a
//! single combined `delay_ms` — jitter and the reorder-hold bump are
//! ADDITIVE contributions to that one number (see `impair.rs`'s own doc
//! comment), so this module has no way to tell, from the `Action` alone,
//! whether a given packet's delay came from a reorder-roll, jitter, or
//! both. Recovering that split would mean either mutating `impair.rs`
//! (out of this module's scope) or re-driving a second `Engine` in
//! lockstep purely to inspect its internal rolls (wasteful and fragile).
//! Instead, `reordered` here counts *observed* out-of-order delivery: as
//! each queued packet is actually sent, this module tracks the highest
//! arrival-order index sent so far, and counts a delivery whose
//! arrival-order index is lower than that high-water mark. That's the
//! wire-visible effect a receiver would actually see, independent of
//! which knob (reorder or jitter variance) caused it — a more honest
//! metric for a proxy whose whole point is producing observable
//! impairment, not just recording internal decisions. A duplicated
//! packet's two copies share one arrival-order index; if both land after
//! the high-water mark has moved past it, both count (a real receiver
//! really does see two additional out-of-order deliveries).
//!
//! # Stats
//!
//! [`ProxyStats`] is written to `stats_json` (if given) every
//! `STATS_INTERVAL` and once more, unconditionally, right before
//! [`run`] returns — atomically (write to a sibling `.tmp` path, then
//! `rename`) so a concurrent reader never observes a half-written file.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::impair::{Action, Engine, ImpairConfig};

/// `recv_from` timeout granularity — short enough that the deadline
/// check and the delayed-send heap drain both run often, long enough to
/// not busy-spin the loop between packets.
const RECV_POLL: Duration = Duration::from_millis(20);

/// How often [`run`] rewrites the stats JSON while the loop is live (it
/// always writes one more time at exit, regardless of this interval).
const STATS_INTERVAL: Duration = Duration::from_secs(10);

/// Recv buffer size — comfortably larger than the largest possible IPv4
/// UDP datagram (65,507 bytes), so a single `recv_from` call can never
/// silently truncate a relayed packet.
const BUF_SIZE: usize = 65536;

/// How long the CURRENT `client_peer` must have gone quiet (no forward-
/// direction datagram from it) before a datagram from a DIFFERENT
/// source is accepted as a legitimate re-learn rather than ignored as a
/// stray/spoofed packet.
///
/// **Why this exists — found empirically, not designed up front.**
/// This proxy's original rule ("learn the client from the first
/// forward-direction datagram, for the process's entire lifetime, full
/// stop") is fine for a single unbroken connection but is fundamentally
/// incompatible with a client that legitimately reconnects: `tst-srt`'s
/// `Socket::connect_with` creates a brand-new libsrt socket handle per
/// call with no explicit local bind, so each reconnect attempt (`send
/// --managed`'s factory, or `recv --managed`'s) uses a FRESH ephemeral
/// UDP source port. Confirmed directly while developing this fix: an
/// 8-second outage window caused the sender's managed reconnect to
/// retry from six DIFFERENT source ports across six attempts, all
/// correctly forwarded to the real destination (forwarding was never
/// gated on `client_peer`) but every REPLY kept routing back to the
/// now-dead original port — the handshake could never complete, no
/// matter how many times either side retried, because the OLD bare
/// `client_peer.get_or_insert(peer)` never updates once set.
///
/// **Why a grace period, not "always re-learn on a new source."**
/// Unconditionally re-aiming the return path on ANY new source would
/// defeat `spoofed_third_party_datagram_does_not_hijack_the_return_
/// path`'s whole point: a stray/malicious datagram from an unrelated
/// third socket must not hijack replies meant for the real client. That
/// test's spoofed datagram arrives with essentially ZERO gap since the
/// real client's own last datagram (same single-threaded test, no sleep
/// in between) — so any grace period meaningfully longer than "in-
/// process instantaneous" and meaningfully shorter than the seconds-
/// scale gap a genuine SRT peer-idle-timeout break produces (confirmed
/// empirically: the break-to-first-reconnect-attempt gap was on the
/// order of several seconds, never sub-second) safely tells the two
/// cases apart. 2 seconds is comfortably inside that gap.
const CLIENT_RELEARN_GRACE: Duration = Duration::from_secs(2);

/// Per-field mirror of [`ImpairConfig`], echoed into [`ProxyStats`].
/// `ImpairConfig` itself doesn't derive `Serialize`/`Deserialize` (this
/// module doesn't modify `impair.rs`), so this is a small hand-written
/// adapter rather than an upstream derive.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ConfigEcho {
    pub loss_pct: f64,
    pub dup_pct: f64,
    pub reorder_pct: f64,
    pub reorder_hold: u32,
    pub jitter_ms_max: u32,
    pub outage_period_s: Option<u64>,
    pub outage_dur_s: u64,
}

impl From<&ImpairConfig> for ConfigEcho {
    fn from(cfg: &ImpairConfig) -> Self {
        ConfigEcho {
            loss_pct: cfg.loss_pct,
            dup_pct: cfg.dup_pct,
            reorder_pct: cfg.reorder_pct,
            reorder_hold: cfg.reorder_hold,
            jitter_ms_max: cfg.jitter_ms_max,
            outage_period_s: cfg.outage_period_s,
            outage_dur_s: cfg.outage_dur_s,
        }
    }
}

/// Running counters + config echo — [`run`]'s return value and the
/// shape written to `stats_json`. See the module doc's "Stats" section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyStats {
    /// Packets decided `Forward` or `DupForward` (i.e. NOT dropped) —
    /// counted once per incoming packet, regardless of duplication (see
    /// `duped` for that).
    pub forwarded: u64,
    /// Packets decided `Drop` (loss roll or outage window).
    pub dropped: u64,
    /// Of the packets counted in `forwarded`, how many were also
    /// duplicated (an extra copy queued and sent).
    pub duped: u64,
    /// Count of actually-observed out-of-order deliveries — see the
    /// module doc's "`reordered`'s definition" section.
    pub reordered: u64,
    pub seed: u64,
    pub config: ConfigEcho,
}

/// One queued (decided-forward, not-yet-due) relayed packet.
struct Delayed {
    due: Instant,
    /// Tie-breaker for heap ordering — see the module doc's "Delayed
    /// forwarding" section.
    seq: u64,
    dest: SocketAddr,
    data: Vec<u8>,
    /// Arrival order of the ORIGINAL incoming packet this entry derives
    /// from. Both copies of a duplicated packet share one `order_key` —
    /// see the module doc's "`reordered`'s definition" section.
    order_key: u64,
}

impl PartialEq for Delayed {
    fn eq(&self, other: &Self) -> bool {
        self.due == other.due && self.seq == other.seq
    }
}
impl Eq for Delayed {}
impl PartialOrd for Delayed {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Delayed {
    fn cmp(&self, other: &Self) -> Ordering {
        self.due
            .cmp(&other.due)
            .then_with(|| self.seq.cmp(&other.seq))
    }
}

/// Queue one delayed copy of `payload`, due `delay_ms` after `now`.
#[allow(clippy::too_many_arguments)]
fn queue_delayed(
    heap: &mut BinaryHeap<Reverse<Delayed>>,
    next_seq: &mut u64,
    dest: SocketAddr,
    payload: &[u8],
    delay_ms: u32,
    order_key: u64,
    now: Instant,
) {
    let seq = *next_seq;
    *next_seq += 1;
    heap.push(Reverse(Delayed {
        due: now + Duration::from_millis(delay_ms as u64),
        seq,
        dest,
        data: payload.to_vec(),
        order_key,
    }));
}

/// Send every heap entry whose `due` has passed (or, if `force`, every
/// remaining entry regardless of `due` — used once at [`run`]'s exit so
/// a packet already counted as forwarded is never silently discarded).
/// Updates `reordered` per the module doc's "`reordered`'s definition"
/// section as each entry is actually sent.
fn drain_ready(
    socket: &UdpSocket,
    heap: &mut BinaryHeap<Reverse<Delayed>>,
    now: Instant,
    force: bool,
    stats: &mut ProxyStats,
    max_sent_order: &mut Option<u64>,
) {
    while let Some(Reverse(top)) = heap.peek() {
        if !force && top.due > now {
            break;
        }
        let Reverse(pkt) = heap.pop().expect("just peeked Some");
        if let Some(high_water) = *max_sent_order {
            if pkt.order_key < high_water {
                stats.reordered += 1;
            }
        }
        *max_sent_order = Some(max_sent_order.map_or(pkt.order_key, |m| m.max(pkt.order_key)));
        // Best-effort: a send failure here (e.g. destination
        // unreachable) is itself indistinguishable from ordinary network
        // loss from the relayed packet's perspective, so it isn't
        // treated as a `run`-level error.
        let _ = socket.send_to(&pkt.data, pkt.dest);
    }
}

fn write_stats_atomic(path: &Path, stats: &ProxyStats) -> Result<(), String> {
    let json = serde_json::to_string_pretty(stats).expect("ProxyStats always serializes");
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp, json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))?;
    Ok(())
}

/// Bind `listen` and relay datagrams to/from `forward` under `cfg`'s
/// impairment, for `run_seconds` seconds (`None` runs until the process
/// is killed — the real CLI's long-running mode). Writes [`ProxyStats`]
/// to `stats_json` (see the module doc's "Stats" section) if given.
///
/// Always prints `{"listening": "<bound addr>"}` to stdout as a single
/// JSON line as soon as the socket is bound (the same discovery pattern
/// `serve::run_hls`/`serve::run_rtsp` use) — this lets a script drive
/// this as a subprocess with `--listen host:0` and learn the actual
/// port. `on_bound`, if given, is additionally invoked (synchronously,
/// right after bind) with the same address — the in-process alternative
/// this crate's own tests use instead of parsing stdout (spawn `run` on
/// a thread, learn the bound port from the callback via a channel).
pub fn run(
    listen: SocketAddr,
    forward: SocketAddr,
    cfg: ImpairConfig,
    stats_json: Option<PathBuf>,
    run_seconds: Option<u64>,
    on_bound: Option<Box<dyn FnOnce(SocketAddr) + Send>>,
) -> Result<ProxyStats, String> {
    let socket = UdpSocket::bind(listen).map_err(|e| format!("proxy bind {listen}: {e}"))?;
    socket
        .set_read_timeout(Some(RECV_POLL))
        .map_err(|e| format!("proxy set_read_timeout: {e}"))?;
    let bound_addr = socket
        .local_addr()
        .map_err(|e| format!("proxy local_addr: {e}"))?;

    println!("{{\"listening\": \"{bound_addr}\"}}");
    if let Some(hook) = on_bound {
        hook(bound_addr);
    }

    let mut engine = Engine::new(cfg);
    let run_start = Instant::now();
    let deadline = run_seconds.map(|s| run_start + Duration::from_secs(s));

    let mut client_peer: Option<SocketAddr> = None;
    // Wall-clock time of the last forward-direction datagram FROM
    // `client_peer` specifically (not from anyone) — see
    // `CLIENT_RELEARN_GRACE`'s own doc comment for what this gates.
    let mut client_last_seen: Option<Instant> = None;
    let mut heap: BinaryHeap<Reverse<Delayed>> = BinaryHeap::new();
    let mut next_seq: u64 = 0;
    let mut next_order: u64 = 0;
    let mut max_sent_order: Option<u64> = None;

    let mut stats = ProxyStats {
        forwarded: 0,
        dropped: 0,
        duped: 0,
        reordered: 0,
        seed: cfg.seed,
        config: ConfigEcho::from(&cfg),
    };

    let mut last_stats_write = Instant::now();
    let mut buf = vec![0u8; BUF_SIZE];

    loop {
        if let Some(dl) = deadline {
            if Instant::now() >= dl {
                break;
            }
        }

        match socket.recv_from(&mut buf) {
            Ok((n, peer)) => {
                let now = Instant::now();
                if peer == forward {
                    // Reverse direction: relay immediately, unimpaired —
                    // see the module doc's "Topology and direction
                    // semantics" section. Nothing to relay to if the
                    // client hasn't been learned yet (shouldn't happen
                    // in practice: `forward` only ever replies to
                    // traffic this proxy itself first sent it).
                    if let Some(client) = client_peer {
                        let _ = socket.send_to(&buf[..n], client);
                    }
                } else {
                    // Forward direction: this is the traffic
                    // `impair::Engine` governs. Learn (or RE-learn, past
                    // a quiet grace period — see `CLIENT_RELEARN_GRACE`)
                    // the client peer from datagrams on this path; see
                    // that constant's own doc comment for why a bare
                    // "first datagram only" rule (this proxy's original
                    // shape) is incompatible with a legitimate client
                    // reconnect.
                    match client_peer {
                        None => {
                            client_peer = Some(peer);
                            client_last_seen = Some(now);
                        }
                        Some(existing) if existing == peer => {
                            client_last_seen = Some(now);
                        }
                        Some(existing) => {
                            let quiet_long_enough = client_last_seen.is_none_or(|seen| {
                                now.duration_since(seen) >= CLIENT_RELEARN_GRACE
                            });
                            if quiet_long_enough {
                                eprintln!(
                                    "proxy: re-learned client peer {peer} (was {existing}, quiet {:?}) \
                                     — treating as a legitimate reconnect, not a spoof",
                                    client_last_seen.map(|seen| now.duration_since(seen))
                                );
                                client_peer = Some(peer);
                                client_last_seen = Some(now);
                            }
                            // else: still within the grace window since
                            // `existing` was last seen — a stray/spoofed
                            // datagram from a third source must not
                            // re-aim the return path (see
                            // `spoofed_third_party_datagram_does_not_
                            // hijack_the_return_path`); the packet is
                            // still relayed below either way, just not
                            // treated as a peer change.
                        }
                    }
                    let elapsed_ms = run_start.elapsed().as_millis() as u64;
                    let order_key = next_order;
                    next_order += 1;
                    match engine.decide(elapsed_ms) {
                        Action::Drop => {
                            stats.dropped += 1;
                        }
                        Action::Forward { delay_ms } => {
                            stats.forwarded += 1;
                            queue_delayed(
                                &mut heap,
                                &mut next_seq,
                                forward,
                                &buf[..n],
                                delay_ms,
                                order_key,
                                now,
                            );
                        }
                        Action::DupForward { delay_ms } => {
                            stats.forwarded += 1;
                            stats.duped += 1;
                            for _ in 0..2 {
                                queue_delayed(
                                    &mut heap,
                                    &mut next_seq,
                                    forward,
                                    &buf[..n],
                                    delay_ms,
                                    order_key,
                                    now,
                                );
                            }
                        }
                    }
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            // Windows: a datagram sent to a peer whose port already
            // closed can surface as ConnectionReset/ConnectionAborted on
            // a LATER, otherwise-unrelated recv_from on this same
            // unconnected socket (Linux just stays silent) — a peer
            // process exiting early must not kill the whole relay over
            // an async ICMP artifact like this.
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
                ) =>
            {
                eprintln!(
                    "proxy: recv_from reported {e} (peer likely closed; continuing to relay)"
                );
            }
            Err(e) => return Err(format!("proxy recv_from: {e}")),
        }

        drain_ready(
            &socket,
            &mut heap,
            Instant::now(),
            false,
            &mut stats,
            &mut max_sent_order,
        );

        if let Some(path) = &stats_json {
            if last_stats_write.elapsed() >= STATS_INTERVAL {
                // A transient write failure (e.g. the containing
                // directory got removed mid-soak) must not abort the
                // relay itself — that would silently stop forwarding
                // real traffic over a problem with an evidence
                // side-channel. Log and keep relaying; the next
                // interval (or the unconditional write at exit) gets
                // another chance.
                if let Err(e) = write_stats_atomic(path, &stats) {
                    eprintln!("proxy: periodic stats write failed (continuing to relay): {e}");
                }
                last_stats_write = Instant::now();
            }
        }
    }

    // Flush every remaining queued packet regardless of its due time —
    // it was already decided (and counted) as forwarded, so silently
    // dropping it here would be an artifact of shutdown timing, not a
    // real impairment decision. See the module doc's "Delayed
    // forwarding" section.
    drain_ready(
        &socket,
        &mut heap,
        Instant::now(),
        true,
        &mut stats,
        &mut max_sent_order,
    );

    if let Some(path) = &stats_json {
        // Same reasoning as the periodic write above: a run that
        // otherwise completed cleanly must not be reported as failed
        // just because its LAST stats write hit a transient FS problem
        // — that would mask a successful relay (and, for the CLI path,
        // exit non-zero) over what's ultimately a side-channel write.
        // The in-process API (this function's return value) still
        // carries the real, final `ProxyStats` regardless.
        if let Err(e) = write_stats_atomic(path, &stats) {
            eprintln!("proxy: final stats write failed (relay itself completed normally): {e}");
        }
    }

    Ok(stats)
}

/// Parse a percent argument (`--loss`/`--dup`/the `PCT` half of
/// `--reorder`), rejecting anything that isn't finite and in
/// `0.0..=100.0` — mirrors `cli::parse_seconds`'s reject-degenerate-
/// shapes style (this module doesn't reuse that function directly: it
/// validates a different range and `cli.rs` isn't otherwise touched by
/// this task).
pub fn parse_percent(s: &str) -> Option<f64> {
    let v: f64 = s.parse().ok()?;
    if v.is_finite() && (0.0..=100.0).contains(&v) {
        Some(v)
    } else {
        None
    }
}

/// Parse `--reorder PCT,HOLD_MS` into `(reorder_pct, reorder_hold_ms)`.
/// `PCT` follows [`parse_percent`]'s validation; `HOLD_MS` is a plain
/// non-negative integer milliseconds delay, passed straight through to
/// [`ImpairConfig::reorder_hold`] — picking a value that corresponds to
/// a few packet intervals for the caller's own traffic rate is the
/// CALLER's job (see that field's own doc comment), not something this
/// parser second-guesses.
pub fn parse_reorder(s: &str) -> Option<(f64, u32)> {
    let (pct_s, hold_s) = s.split_once(',')?;
    let pct = parse_percent(pct_s)?;
    let hold: u32 = hold_s.parse().ok()?;
    Some((pct, hold))
}

/// Parse an integer with an optional trailing `h`/`m`/`s` unit suffix
/// (seconds if omitted) into a plain second count.
fn parse_duration_unit(s: &str) -> Option<u64> {
    let (digits, mult) = if let Some(d) = s.strip_suffix('h') {
        (d, 3600)
    } else if let Some(d) = s.strip_suffix('m') {
        (d, 60)
    } else if let Some(d) = s.strip_suffix('s') {
        (d, 1)
    } else {
        (s, 1)
    };
    let n: u64 = digits.parse().ok()?;
    n.checked_mul(mult)
}

/// Parse `--outage period=DUR,dur=DUR` (each `DUR` an integer with an
/// optional `h`/`m`/`s` unit suffix — see `parse_duration_unit`) into
/// `(outage_period_s, outage_dur_s)`. Both keys are required whenever
/// `--outage` is passed at all: a period with no duration (or vice
/// versa) would silently parse into `ImpairConfig{ outage_period_s:
/// None, .. }` — indistinguishable from no outage configured at all
/// (`Engine::in_outage` short-circuits on `None` regardless of
/// `outage_dur_s`) — which would make a caller's typo (e.g. forgetting
/// `period=`) fail silently instead of erroring. Omit the whole
/// `--outage` flag for "no outage" (`ImpairConfig`'s own default); the
/// caller maps this function's `Some((period, dur))` to
/// `outage_period_s: Some(period)`.
pub fn parse_outage(s: &str) -> Option<(u64, u64)> {
    let mut period: Option<u64> = None;
    let mut dur: Option<u64> = None;
    for part in s.split(',') {
        let (key, val) = part.split_once('=')?;
        match key {
            "period" => period = Some(parse_duration_unit(val)?),
            "dur" => dur = Some(parse_duration_unit(val)?),
            _ => return None,
        }
    }
    Some((period?, dur?))
}

/// Parse a `--run-seconds` argument, rejecting anything that isn't a
/// strictly positive integer (mirrors `cli::parse_seconds`'s reject-
/// degenerate-shapes style for the integer/duration case).
pub fn parse_run_seconds(s: &str) -> Option<u64> {
    let v: u64 = s.parse().ok()?;
    if v > 0 { Some(v) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_percent_accepts_the_valid_range() {
        assert_eq!(parse_percent("0"), Some(0.0));
        assert_eq!(parse_percent("2.5"), Some(2.5));
        assert_eq!(parse_percent("100"), Some(100.0));
    }

    #[test]
    fn parse_percent_rejects_out_of_range_and_degenerate_values() {
        assert_eq!(parse_percent("-1"), None);
        assert_eq!(parse_percent("100.1"), None);
        assert_eq!(parse_percent("NaN"), None);
        assert_eq!(parse_percent("inf"), None);
        assert_eq!(parse_percent("not-a-number"), None);
    }

    #[test]
    fn parse_reorder_splits_pct_and_hold_ms() {
        assert_eq!(parse_reorder("1,3"), Some((1.0, 3)));
        assert_eq!(parse_reorder("0.5,200"), Some((0.5, 200)));
    }

    #[test]
    fn parse_reorder_rejects_malformed_input() {
        assert_eq!(parse_reorder("1"), None);
        assert_eq!(parse_reorder("101,3"), None);
        assert_eq!(parse_reorder("1,-3"), None);
        assert_eq!(parse_reorder(""), None);
    }

    #[test]
    fn parse_outage_handles_unit_suffixes() {
        assert_eq!(parse_outage("period=6h,dur=90s"), Some((21_600, 90)));
        assert_eq!(parse_outage("period=2s,dur=1s"), Some((2, 1)));
        assert_eq!(parse_outage("period=5m,dur=30"), Some((300, 30)));
    }

    #[test]
    fn parse_outage_rejects_missing_or_unknown_keys() {
        assert_eq!(parse_outage("period=6h"), None); // dur required
        assert_eq!(parse_outage("dur=90s"), None); // period required
        assert_eq!(parse_outage("bogus=1,dur=2"), None);
        assert_eq!(parse_outage(""), None);
    }

    #[test]
    fn parse_run_seconds_rejects_zero_and_negative() {
        assert_eq!(parse_run_seconds("5"), Some(5));
        assert_eq!(parse_run_seconds("0"), None);
        assert_eq!(parse_run_seconds("-1"), None);
        assert_eq!(parse_run_seconds("abc"), None);
    }
}

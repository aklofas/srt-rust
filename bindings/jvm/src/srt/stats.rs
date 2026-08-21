//! Build the `org.tstrans.srt.SocketStats` / `SrtStats` Java records from the
//! Rust stats structs. All integer counters widen to `i64` (Java has no unsigned
//! types — reinterpret the bit pattern; documented in the Java records). The `f64`
//! field `mbpsEstimatedBandwidth` passes through as a JNI `jdouble`.

use jni::JNIEnv;
use jni::objects::JObject;
use jni::objects::JValue;
use tst_core::transport::SocketStats;

/// Build an `org.tstrans.srt.SocketStats` record from a Rust `SocketStats`.
/// Field order matches the Java record ctor exactly (16 longs, in declaration
/// order): rttUs, sendBandwidthBps, recvBandwidthBps, linkBandwidthBps,
/// bytesSent, packetsSent, bytesReceived, packetsReceived, bytesLostRecv,
/// packetsLostRecv, packetsLostSend, packetsRetransmitted, packetsDroppedSend,
/// packetsDroppedRecv, sendBufferPackets, recvBufferPackets.
pub(crate) fn build_socket_stats<'local>(
    env: &mut JNIEnv<'local>,
    s: &SocketStats,
) -> jni::errors::Result<JObject<'local>> {
    env.ensure_local_capacity(4)?;
    let sig = "(JJJJJJJJJJJJJJJJ)V"; // 16 longs
    env.new_object(
        "org/tstrans/srt/SocketStats",
        sig,
        &[
            JValue::Long(i64::from(s.rtt_us)),
            JValue::Long(s.send_bandwidth_bps as i64),
            JValue::Long(s.recv_bandwidth_bps as i64),
            JValue::Long(s.link_bandwidth_bps as i64),
            JValue::Long(s.bytes_sent as i64),
            JValue::Long(s.packets_sent as i64),
            JValue::Long(s.bytes_received as i64),
            JValue::Long(s.packets_received as i64),
            JValue::Long(s.bytes_lost_recv as i64),
            JValue::Long(s.packets_lost_recv as i64),
            JValue::Long(s.packets_lost_send as i64),
            JValue::Long(s.packets_retransmitted as i64),
            JValue::Long(s.packets_dropped_send as i64),
            JValue::Long(s.packets_dropped_recv as i64),
            JValue::Long(i64::from(s.send_buffer_packets)),
            JValue::Long(i64::from(s.recv_buffer_packets)),
        ],
    )
}

/// Build an `org.tstrans.srt.SrtStats` record from a Rust `tst_srt::Stats`.
/// Field order matches the Java record ctor exactly (16 longs + 1 double):
/// bytesSent, bytesReceived, bytesLostRecvSide, bytesLostSendSide,
/// packetsSent, packetsReceived, packetsLostRecvSide, packetsLostSendSide,
/// packetsRetransmitted, packetsDroppedRecvSide, packetsDroppedSendSide,
/// rttUs, sendBandwidthBps, recvBandwidthBps, mbpsEstimatedBandwidth (D),
/// sendBufferPackets, recvBufferPackets.
/// `rtt` is a `Duration`; saturate to `u32::MAX` µs before widening to `i64`.
pub(crate) fn build_srt_stats<'local>(
    env: &mut JNIEnv<'local>,
    s: &tst_srt::Stats,
) -> jni::errors::Result<JObject<'local>> {
    env.ensure_local_capacity(4)?;
    let rtt_us = u32::try_from(s.rtt.as_micros()).unwrap_or(u32::MAX);
    let sig = "(JJJJJJJJJJJJJJDJJ)V"; // 14 longs, 1 double, 2 longs
    env.new_object(
        "org/tstrans/srt/SrtStats",
        sig,
        &[
            JValue::Long(s.bytes_sent as i64),
            JValue::Long(s.bytes_received as i64),
            JValue::Long(s.bytes_lost_recv_side as i64),
            JValue::Long(s.bytes_lost_send_side as i64),
            JValue::Long(s.packets_sent as i64),
            JValue::Long(s.packets_received as i64),
            JValue::Long(s.packets_lost_recv_side as i64),
            JValue::Long(s.packets_lost_send_side as i64),
            JValue::Long(s.packets_retransmitted as i64),
            JValue::Long(s.packets_dropped_recv_side as i64),
            JValue::Long(s.packets_dropped_send_side as i64),
            JValue::Long(i64::from(rtt_us)),
            JValue::Long(s.send_bandwidth_bps as i64),
            JValue::Long(s.recv_bandwidth_bps as i64),
            JValue::Double(s.mbps_estimated_bandwidth),
            JValue::Long(i64::from(s.send_buffer_packets)),
            JValue::Long(i64::from(s.recv_buffer_packets)),
        ],
    )
}

/// Build an `org.tstrans.srt.ManagedTransportStats` record from a Rust
/// `tst_pipeline::ManagedTransportStats`. Field order matches the Java record
/// ctor exactly (5 longs + 1 bool, in declaration order): reconnectAttempts,
/// reconnectSuccesses, gapLen, gapMessagesDropped, gapBytesDropped, reconnecting.
pub(crate) fn build_managed_transport_stats<'local>(
    env: &mut JNIEnv<'local>,
    s: &tst_pipeline::ManagedTransportStats,
) -> jni::errors::Result<JObject<'local>> {
    env.ensure_local_capacity(4)?;
    let sig = "(JJJJJZ)V"; // 5 longs, 1 bool
    env.new_object(
        "org/tstrans/srt/ManagedTransportStats",
        sig,
        &[
            JValue::Long(s.reconnect_attempts as i64),
            JValue::Long(s.reconnect_successes as i64),
            JValue::Long(s.gap_len as i64),
            JValue::Long(s.gap_messages_dropped as i64),
            JValue::Long(s.gap_bytes_dropped as i64),
            JValue::Bool(u8::from(s.reconnecting)),
        ],
    )
}

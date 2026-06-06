//! Build the `org.tstrans.rtp.SocketStats` Java record from the Rust
//! `tst_core::transport::SocketStats`. All counters widen to `i64` (Java has no
//! unsigned types — the bit pattern is reinterpreted; documented in the record).

use jni::JNIEnv;
use jni::objects::{JObject, JValue};
use tst_core::transport::SocketStats;

/// Build an `org.tstrans.rtp.SocketStats` record. Field order matches the Java
/// record ctor exactly (16 longs, in declaration order): rttUs, sendBandwidthBps,
/// recvBandwidthBps, linkBandwidthBps, bytesSent, packetsSent, bytesReceived,
/// packetsReceived, bytesLostRecv, packetsLostRecv, packetsLostSend,
/// packetsRetransmitted, packetsDroppedSend, packetsDroppedRecv,
/// sendBufferPackets, recvBufferPackets.
pub(crate) fn build_socket_stats<'local>(
    env: &mut JNIEnv<'local>,
    s: &SocketStats,
) -> jni::errors::Result<JObject<'local>> {
    env.ensure_local_capacity(4)?;
    let sig = "(JJJJJJJJJJJJJJJJ)V"; // 16 longs
    env.new_object(
        "org/tstrans/rtp/SocketStats",
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

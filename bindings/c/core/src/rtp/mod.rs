//! `tst_rtp_*` C ABI entry points. Gated on `feature = "rtp"`.
//!
//! This module exposes constructors that open RTP transports and return
//! new opaque handle types (`TstRtpSender`, `TstRtpReceiver`,
//! `TstRtpMuxSender`, `TstRtpDemuxReceiver`). Once open, callers use the
//! handle-specific data-path entry points in the sub-modules below.
//! Each handle has its own `_close` entry point to free it.
//!
//! URL form accepted: `rtp://host:port[?key=value&...]`
//! See `tst_rtp::RtpUrl` for the recognized query keys (ttl, iface,
//! pkt_size, ssrc).

pub(crate) mod url;

pub mod sender;
pub mod receiver;
pub mod mux_sender;
pub mod demux_receiver;

pub use demux_receiver::{
    TstRtpDemuxReceiver, tst_rtp_demux_receiver_cancel, tst_rtp_demux_receiver_close,
    tst_rtp_demux_receiver_get_socket_stats, tst_rtp_demux_receiver_get_stats,
    tst_rtp_demux_receiver_get_stream_codec_stats, tst_rtp_demux_receiver_get_stream_stats,
    tst_rtp_demux_receiver_next_event, tst_rtp_demux_receiver_open,
    tst_rtp_demux_receiver_reset_stats,
};
pub use mux_sender::{
    TstRtpMuxSender, tst_rtp_mux_sender_cancel, tst_rtp_mux_sender_close,
    tst_rtp_mux_sender_get_mux_sender_stats, tst_rtp_mux_sender_get_socket_stats,
    tst_rtp_mux_sender_get_stream_codec_stats, tst_rtp_mux_sender_open,
    tst_rtp_mux_sender_push_audio, tst_rtp_mux_sender_push_audio_to, tst_rtp_mux_sender_push_klv,
    tst_rtp_mux_sender_push_klv_to, tst_rtp_mux_sender_push_subtitle,
    tst_rtp_mux_sender_push_subtitle_to, tst_rtp_mux_sender_push_video,
    tst_rtp_mux_sender_push_video_to, tst_rtp_mux_sender_reset_stats,
};
pub use receiver::{
    TstRtpReceiver, tst_rtp_receiver_cancel, tst_rtp_receiver_close,
    tst_rtp_receiver_get_socket_stats, tst_rtp_receiver_get_stats, tst_rtp_receiver_recv_ts,
    tst_rtp_receiver_reset_stats, tst_rtp_recv_open,
};
pub use sender::{
    TstRtpSender, tst_rtp_sender_cancel, tst_rtp_sender_close, tst_rtp_sender_get_socket_stats,
    tst_rtp_sender_get_stats, tst_rtp_sender_open, tst_rtp_sender_reset_stats,
    tst_rtp_sender_send_ts,
};

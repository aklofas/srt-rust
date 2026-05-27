//! HLS publisher — segments MPEG-TS to disk + serves a built-in HTTP playlist.
//!
//! See [`HlsPublisher`] for the entry point. Modes: LIVE (rolling window),
//! EVENT (monotone-growing + ENDLIST on finish), VOD (all segments at once
//! + ENDLIST on finish). KLV stays inside the .ts segments.

#![warn(rustdoc::broken_intra_doc_links)]

pub mod builder;
pub mod config;
pub mod error;
pub mod publisher;
pub mod stats;
pub mod url;

mod auth;
mod http_server;
mod playlist;
mod segmenter;
#[cfg(feature = "tls")]
mod tls;

// pub use lines uncommented as each phase lands its types.
pub use builder::HlsPublisherBuilder;
pub use config::{HlsConfig, HlsMode};
pub use error::{HlsError, HlsErrorKind};
pub use publisher::HlsPublisher;
pub use stats::HlsStats;
pub use url::{HlsUrl, HlsUrlError};

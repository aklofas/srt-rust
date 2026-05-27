//! Safe Rust wrapper around librist 0.2.x.
//!
//! - Sender: [`transport::RistTransport`] (impl [`tst_core::transport::Transport`])
//! - Receiver: [`recv::RistRecvTransport`] (impl [`tst_core::transport::RecvTransport`])
//! - Profiles: [`config::RistProfile::Simple`] and [`config::RistProfile::Main`]
//!   (Advanced and Auth deferred via `#[non_exhaustive]`)
//! - Encryption: AES-128 / 192 / 256 PSK behind the `mbedtls` cargo feature
//!   (default-on)
//!
//! URL forms:
//! - `rist://host:port` — Simple Profile sender (unicast UDP)
//! - `rist://@host:port` — receiver bind (ffmpeg `@` convention; same trick as
//!   `tst-udp` / `tst-tcp`)
//! - `rist://239.x.x.x:port` — multicast sender
//! - `?profile=simple|main` — explicit profile override
//! - `?bandwidth=N` — kbps target
//! - `?buffer=N` — recovery buffer ms
//! - `?aes-type=128|192|256&secret=...` — AES key (forces Main Profile)
//! - `?cname=...` — RTCP CNAME

#![warn(rustdoc::broken_intra_doc_links)]

pub mod builder;
pub mod config;
pub mod error;
pub mod init;
pub mod recv;
pub mod stats;
pub mod transport;
pub mod url;

// Re-exports populated as types land. Remaining lines uncommented as Waves
// B-E land their modules.
pub use config::{EncryptionKey, RistConfig, RistProfile};
pub use error::{RistError, RistErrorKind};
pub use stats::RistStats;
pub use url::{RistUrl, RistUrlError};

pub use recv::RistRecvTransport;
pub use transport::RistTransport;

// pub use builder::{RistRecvTransportBuilder, RistTransportBuilder};  // Wave E

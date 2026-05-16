//! Strip parameter-set NAL/OBU bytes from official codec conformance
//! bitstreams. Driven by `manifest.toml`. See `plan
//! 2026-05-15-codec-conformance-bitstreams.md` for design.

// Stub: scanner + downloader land in later tasks; types/functions defined now
// for the manifest loader unit tests and to lock the TOML schema.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    h264: Vec<Entry>,
    #[serde(default)]
    h265: Vec<Entry>,
    #[serde(default)]
    h266: Vec<Entry>,
    #[serde(default)]
    av1: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    name: String,
    archive_url: String,
    /// Path within the archive. Empty string => archive_url IS the raw bitstream.
    #[serde(default)]
    extract: String,
    sha256: String,
    kind: String,
    #[serde(default)]
    nal_index: u32,
    #[serde(default)]
    obu_index: u32,
    expected: Expected,
}

#[derive(Debug, Deserialize, Serialize)]
struct Expected {
    outcome: String, // "ok" or "err"
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<BTreeMap<String, toml::Value>>,
}

fn load_manifest(path: &std::path::Path) -> Result<Manifest, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    toml::from_str(&s).map_err(|e| format!("parse {}: {}", path.display(), e))
}

fn main() {
    eprintln!("not yet implemented");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn manifest_loads_starter_set() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/conformance/manifest.toml");
        let m = load_manifest(&path).expect("starter manifest must parse");
        assert!(m.h264.len() >= 3, "h264 entries: {}", m.h264.len());
        assert!(m.h265.len() >= 3, "h265 entries: {}", m.h265.len());
        assert!(m.h266.len() >= 3, "h266 entries: {}", m.h266.len());
        assert!(m.av1.len() >= 2, "av1 entries: {}", m.av1.len());
    }

    #[test]
    fn manifest_entry_round_trips_required_fields() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/conformance/manifest.toml");
        let m = load_manifest(&path).expect("parse");
        for e in &m.h264 {
            assert!(!e.name.is_empty());
            assert!(!e.archive_url.is_empty());
            assert_eq!(e.sha256.len(), 64, "sha256 hex must be 64 chars");
            assert_eq!(e.kind, "h264_sps");
        }
    }
}

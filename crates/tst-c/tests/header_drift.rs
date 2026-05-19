//! Verifies that the committed header matches what cbindgen would emit
//! against the current source tree, plus the build.rs post-process step
//! that inserts domain-grouping section dividers (audit Finding 5).
//! Drift indicates a forgotten regenerate-and-commit step.

use std::fs;
use std::path::PathBuf;

#[test]
fn committed_header_matches_cbindgen_output() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let committed = PathBuf::from(manifest_dir).join("include/tstrans.h");
    let expected = fs::read_to_string(&committed).expect("read committed tstrans.h");

    let config = cbindgen::Config::from_file(format!("{manifest_dir}/cbindgen.toml"))
        .expect("read cbindgen.toml");
    let generated = cbindgen::Builder::new()
        .with_config(config)
        .with_crate(manifest_dir)
        .generate()
        .expect("cbindgen generate");
    let mut buf = Vec::new();
    generated.write(&mut buf);
    let generated = String::from_utf8(buf).expect("cbindgen output is utf-8");

    // Mirror the build.rs post-process so this test compares apples to
    // apples (the committed header is cbindgen output + section dividers).
    let generated = add_section_dividers(&generated);

    if generated != expected {
        eprintln!(
            "Drift between crates/tst-c/include/tstrans.h and cbindgen output. \
             Run `cargo build -p tst-c && cp target/debug/include/tstrans.h crates/tst-c/include/tstrans.h` \
             and commit the diff."
        );
        // Compute a small diff hint: first differing line.
        for (i, (g, e)) in generated.lines().zip(expected.lines()).enumerate() {
            if g != e {
                eprintln!("first diff at line {}:", i + 1);
                eprintln!("  expected: {e}");
                eprintln!("  generated: {g}");
                break;
            }
        }
        panic!("header drift");
    }
}

/// Mirror of `build.rs::add_section_dividers`. Kept in lock-step with
/// build.rs by convention (if you change one, change the other). Both
/// duplicate the small helper logic rather than depending on a shared
/// module — build.rs runs before the crate compiles, so it can't import
/// from a `tst_c::` path.
fn add_section_dividers(original: &str) -> String {
    let sections: &[(&[&str], &str)] = &[
        (
            &[
                "tst_get_version_",
                "tst_get_abi_version_",
                "tst_get_last_error",
                "tst_clear_last_error",
            ],
            "INTROSPECTION",
        ),
        (
            &[
                "tst_mux_sender_",
                "tst_managed_mux_sender_",
                "tst_mux_config_",
                "tst_muxer_",
            ],
            "MUX SENDER",
        ),
        (&["tst_sender_", "tst_managed_sender_"], "TS SENDER"),
        (
            &["tst_raw_sender_", "tst_managed_raw_sender_"],
            "RAW SENDER",
        ),
        (
            &[
                "tst_demux_receiver_",
                "tst_managed_demux_receiver_",
                "tst_demux_config_",
            ],
            "DEMUX RECEIVER",
        ),
        (&["tst_receiver_", "tst_managed_receiver_"], "TS RECEIVER"),
        (
            &["tst_raw_receiver_", "tst_managed_raw_receiver_"],
            "RAW RECEIVER",
        ),
    ];

    let mut out = String::with_capacity(original.len() + 1024);
    let mut prev_section: Option<&str> = None;

    for line in original.lines() {
        let symbol = extract_function_symbol(line);
        if let Some(sym) = symbol {
            let section = classify_symbol(sym, sections);
            if Some(section) != prev_section {
                out.push_str(&format!(
                    "\n// ─── {} {}\n",
                    section,
                    "─".repeat(60usize.saturating_sub(section.len() + 6))
                ));
                prev_section = Some(section);
            }
        }
        out.push_str(line);
        out.push('\n');
    }

    out
}

fn extract_function_symbol(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('*') || trimmed.starts_with("/*") || trimmed.starts_with("//") {
        return None;
    }
    let paren = line.find('(')?;
    let head = &line[..paren];
    let tst_start = head.rfind("tst_")?;
    if tst_start > 0 {
        let prev = head.as_bytes()[tst_start - 1];
        if !(prev == b' ' || prev == b'\t' || prev == b'*') {
            return None;
        }
    }
    let symbol = &head[tst_start..];
    if symbol
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        Some(symbol)
    } else {
        None
    }
}

fn classify_symbol(symbol: &str, sections: &[(&[&str], &str)]) -> &'static str {
    for (prefixes, name) in sections {
        for prefix in *prefixes {
            if symbol.starts_with(prefix) {
                return match *name {
                    "INTROSPECTION" => "INTROSPECTION",
                    "MUX SENDER" => "MUX SENDER",
                    "TS SENDER" => "TS SENDER",
                    "RAW SENDER" => "RAW SENDER",
                    "DEMUX RECEIVER" => "DEMUX RECEIVER",
                    "TS RECEIVER" => "TS RECEIVER",
                    "RAW RECEIVER" => "RAW RECEIVER",
                    _ => "OTHER",
                };
            }
        }
    }
    if symbol.ends_with("_close") || symbol.ends_with("_cancel") || symbol.ends_with("_free") {
        "LIFETIME"
    } else {
        "OTHER"
    }
}

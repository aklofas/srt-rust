//! Runs cbindgen to generate `target/<profile>/include/tstrans.h`.

use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");

    // The header is emitted into target/<profile>/include/tstrans.h, which is a
    // sibling-of-OUT_DIR layout suitable for downstream consumers to find on
    // their include path. OUT_DIR is .../target/<profile>/build/tst-c-<hash>/out;
    // walk three levels up to get .../target/<profile>.
    let profile_dir = PathBuf::from(&out_dir)
        .ancestors()
        .nth(3)
        .expect("OUT_DIR ancestor walk failed")
        .to_path_buf();
    let include_dir = profile_dir.join("include");
    std::fs::create_dir_all(&include_dir).expect("create include dir");
    let header_path = include_dir.join("tstrans.h");

    let config =
        cbindgen::Config::from_file(format!("{crate_dir}/cbindgen.toml")).expect("cbindgen.toml");

    cbindgen::Builder::new()
        .with_config(config)
        .with_crate(&crate_dir)
        .generate()
        .expect("cbindgen generate")
        .write_to_file(&header_path);

    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=src");

    // Post-process: domain-grouping section dividers (audit Finding 5).
    // Keys on symbol-name prefix; independent of Plan C's tst-c/src/
    // reorg (cbindgen is symbol-based, not file-based, so symbol ordering
    // stays stable across source-tree restructuring).
    add_section_dividers(&header_path);

    // ────────────────────────────────────────────────────────────────
    // Symbol hygiene (audit 09-c-abi.md Finding 3): restrict the
    // libtstrans dynamic export table to tst_*/TST_* symbols on
    // platforms that support per-symbol linker-script export gates.
    // Linux uses --exclude-libs=ALL (hides all static-library symbols
    // from the dynamic export table; libsrt/mbedTLS are statically
    // linked so their srt_*/SRT_*/mbedtls_* exports are dropped while
    // our own #[no_mangle] tst_* symbols remain).
    // macOS uses -exported_symbols_list (whitelist by symbol-name
    // pattern). Windows MSVC is deferred to plan #65's follow-up
    // (runtime tests blocked on Windows hardware — see
    // project_plan_65 memory entry).
    //
    // Note: Plan B originally specified -Wl,--version-script=... for
    // Linux, but that conflicts with rustc's auto-emitted anonymous
    // version-script for cdylib targets (GNU BFD ld rejects mixing
    // anonymous and named version tags). --exclude-libs=ALL achieves
    // the same outcome (0 srt_*/SRT_* in libtstrans.so's export table)
    // without touching the auto-emitted script.
    // ────────────────────────────────────────────────────────────────

    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-link-arg=-Wl,--exclude-libs=ALL");
    }

    #[cfg(target_os = "macos")]
    {
        let crate_dir_path = PathBuf::from(&crate_dir);
        let exports_path = crate_dir_path.join("exports.txt");
        println!("cargo:rerun-if-changed=exports.txt");
        println!(
            "cargo:rustc-link-arg=-Wl,-exported_symbols_list,{}",
            exports_path.display()
        );
    }

    #[cfg(target_os = "windows")]
    {
        // Defer: Windows MSVC linker uses .def files (/DEF:foo.def) or
        // per-symbol /EXPORT: args; both are mechanically straightforward
        // but runtime testing is blocked on Windows hardware. When the
        // plan #65 deferral lifts, ship a tst-c.def and add:
        //   println!("cargo:rerun-if-changed=tst-c.def");
        //   println!("cargo:rustc-link-arg=/DEF:tst-c.def");
        // For now: compile+link still works (no export-restriction means
        // all symbols remain exported, matching the pre-Plan-B Linux/macOS
        // behavior — this is the current Windows compile+link-only
        // status per plan #65).
    }

    // pkg-config substitution.
    let version = env!("CARGO_PKG_VERSION");
    let template_path = PathBuf::from(&crate_dir).join("tstrans.pc.in");
    let template = std::fs::read_to_string(&template_path).expect("read tstrans.pc.in");
    let pc = template
        .replace("@VERSION@", version)
        .replace("@PREFIX@", "/usr/local"); // tarball install default; consumer can sed it
    let pc_path = profile_dir.join("tstrans.pc");
    std::fs::write(&pc_path, pc).expect("write tstrans.pc");
    println!("cargo:rerun-if-changed=tstrans.pc.in");
}

/// Rewrite the cbindgen-generated header in place, inserting domain-
/// grouping section-divider comments before each block of function
/// declarations matching a known prefix.
///
/// Per audit 09-c-abi.md Finding 5. Keys on symbol-name prefix (not
/// source-file location), so Plan C's tst-c/src/ reorg doesn't affect
/// the grouping. Section order in the header follows the table below.
///
/// 7 required domain sections (always emit) + 2 conditional catch-alls
/// (emit only when their bucket is non-empty):
///
/// Required:
/// - INTROSPECTION: version / last-error / clear
/// - MUX SENDER:    tst_mux_sender / tst_managed_mux_sender / tst_mux_config / tst_muxer
/// - TS SENDER:     tst_sender / tst_managed_sender
/// - RAW SENDER:    tst_raw_sender / tst_managed_raw_sender
/// - DEMUX RECEIVER: tst_demux_receiver / tst_managed_demux_receiver / tst_demux_config
/// - TS RECEIVER:   tst_receiver / tst_managed_receiver
/// - RAW RECEIVER:  tst_raw_receiver / tst_managed_raw_receiver
///
/// Conditional (catch-alls; emit only when at least one symbol matched):
/// - LIFETIME:      *_close / *_cancel / *_free (catch-all cleanup)
/// - OTHER:         catch-all safety net
fn add_section_dividers(header_path: &std::path::Path) {
    let original = std::fs::read_to_string(header_path).expect("read tstrans.h");

    // (prefix patterns, section name) in match-order. First match wins.
    // Order: INTROSPECTION first (most-specific tst_get_*); domain sections
    // in sender-then-receiver order matching the README crate-graph; LIFETIME
    // is a catch-all suffix matcher; OTHER catches anything unclassified.
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

    std::fs::write(header_path, out).expect("write tstrans.h with dividers");
}

/// Extract a leading `tst_<word>` symbol from a function-declaration line.
/// Returns None for lines that don't look like a function declaration.
///
/// Filters out doc-comment continuation lines (those starting with ` *` or
/// `/*`) — cbindgen emits doxygen comments that contain backtick-wrapped
/// references like `` `tst_get_last_error()` ``, which would otherwise
/// false-match here and cause spurious section dividers to land inside
/// comment blocks.
fn extract_function_symbol(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('*') || trimmed.starts_with("/*") || trimmed.starts_with("//") {
        return None;
    }
    let paren = line.find('(')?;
    let head = &line[..paren];
    let tst_start = head.rfind("tst_")?;
    // Reject if `tst_` is preceded by a backtick (inline-code in a doc
    // comment) or any other non-whitespace, non-start character that
    // signals this isn't a declaration prefix.
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

/// Classify a `tst_*` symbol into one of the 7 required + 2 conditional
/// sections. Match order = first match wins. LIFETIME and OTHER are tried
/// only if no domain section matches.
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

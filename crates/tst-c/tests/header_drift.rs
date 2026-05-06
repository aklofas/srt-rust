//! Verifies that the committed header matches what cbindgen would emit
//! against the current source tree. Drift indicates a forgotten
//! regenerate-and-commit step.

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

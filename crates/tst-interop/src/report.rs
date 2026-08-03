//! `report merge` / `report render`: turn per-cell interop results into
//! the published evidence artifact.
//!
//! The orchestrator (a later task's `run-matrix.sh`) writes one JSON file
//! per cell into a directory. [`merge`] reads all of them, checks each
//! `FAIL` against a hand-parsed expectations file, and writes a single
//! `results.json` ([`Results`]). [`render`] turns that `results.json`
//! into a markdown report.
//!
//! **The load-bearing property: an unexpected failure can never be
//! silently absorbed.** A `FAIL` only becomes non-fatal
//! ([`Verdict::ExpectedUnsupported`]) when it matches a specific,
//! human-authored entry in the expectations file. Everything else that
//! fails stays [`Verdict::Fail`], which is what [`merge`]'s caller checks
//! (`results.summary.fail > 0`) to decide the process exit code — there
//! is no other path to a passing run.
//!
//! Expectations are read from a small hand-rolled TOML-shaped format (see
//! [`parse_expectations`]) rather than a real TOML crate — the workspace
//! has no `toml` dependency in normal deps and this crate doesn't widen
//! that set for a machine-written file with an exact, fixed schema.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::report_types::CellMetrics;

/// Axis-group row count above which [`render_markdown`] wraps the group
/// in a collapsed `<details>` block instead of a bare `##` heading.
const AXIS_DETAILS_THRESHOLD: usize = 15;

/// Outcome an orchestrator observed for one cell, before expectations are
/// applied. See [`RawCell`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RawVerdict {
    Pass,
    Fail,
    SkippedToolMissing,
}

/// One per-cell JSON file as written by the orchestrator into
/// `--cells-dir`. Field shapes mirror the orchestrator's contract
/// verbatim (see the task brief's Interfaces block) — `metrics` is
/// `null` for a `SkippedToolMissing` cell and for any cell whose peer
/// tool produced no parseable capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCell {
    pub id: String,
    pub profile: String,
    pub peer: String,
    pub direction: String,
    pub tier: String,
    pub verdict: RawVerdict,
    #[serde(default)]
    pub failures: Vec<String>,
    pub metrics: Option<CellMetrics>,
    pub log: String,
}

/// Final, expectations-applied verdict for one cell — the value
/// [`merge`]'s caller acts on. Exactly these four values; `known_flaky`
/// (see [`MergedCell::known_flaky`]) is a display-only refinement of
/// `ExpectedUnsupported`, not a fifth verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Pass,
    Fail,
    ExpectedUnsupported,
    SkippedToolMissing,
}

/// One cell after expectations matching, as written into `results.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedCell {
    pub id: String,
    pub profile: String,
    pub peer: String,
    pub direction: String,
    pub tier: String,
    pub verdict: Verdict,
    /// Set iff `verdict == ExpectedUnsupported` and the matching
    /// expectation's declared verdict was `known_flaky` rather than
    /// `expected_unsupported` — [`render_markdown`] labels such a row
    /// `KNOWN-FLAKY` instead of `EXPECTED-UNSUPPORTED`.
    pub known_flaky: bool,
    pub failures: Vec<String>,
    pub metrics: Option<CellMetrics>,
    pub log: String,
    /// The matching expectation's `reason`, if `verdict == ExpectedUnsupported`.
    pub expectation_reason: Option<String>,
    /// The matching expectation's optional `ref`, if any.
    pub expectation_ref: Option<String>,
}

/// One expectation whose matched cell turned out to `PASS` — the
/// expectation no longer reproduces and should probably be removed.
/// Only `expected_unsupported` expectations can go stale this way; a
/// `known_flaky` expectation matching a passing cell is normal (a flaky
/// failure that didn't reproduce this run) and is never reported here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleExpectation {
    pub cell: String,
    pub profile: String,
    pub reason: String,
}

/// Verdict tallies plus the stale-expectation list, over one merged run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub total: usize,
    pub pass: usize,
    /// Cells whose `FAIL` matched no expectation — see the module doc's
    /// load-bearing property. `merge`'s caller exits nonzero iff this is
    /// nonzero.
    pub fail: usize,
    pub expected_unsupported: usize,
    pub skipped_tool_missing: usize,
    pub stale_expectations: Vec<StaleExpectation>,
}

/// The full `results.json` document: [`merge`]'s output and
/// [`render`]'s input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Results {
    /// The `--meta` file's contents, embedded verbatim (run date, host,
    /// tool versions — the orchestrator's shape, not this module's).
    pub meta: serde_json::Value,
    pub cells: Vec<MergedCell>,
    pub summary: Summary,
}

/// One `[[expect]]` block from the expectations file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expectation {
    /// Exact cell id, or a pattern ending in `*` (prefix match).
    pub cell: String,
    pub profile: String,
    pub verdict: ExpectVerdict,
    pub reason: String,
    pub reference: Option<String>,
    /// Optional substring narrowing: when set, this expectation only
    /// matches a `FAIL` whose failures text contains it (see the
    /// private `find_expectation` helper's doc comment for the exact
    /// matching rule). Never applied to a `PASS` staleness lookup — an
    /// expectation is checked for staleness by `(cell, profile)` alone,
    /// regardless of what failure text it was originally written to
    /// match.
    pub failure_contains: Option<String>,
}

/// The two verdicts an expectation can declare. See
/// [`MergedCell::known_flaky`] for how `KnownFlaky` differs from
/// `ExpectedUnsupported` in the merged output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectVerdict {
    ExpectedUnsupported,
    KnownFlaky,
}

// ---------------------------------------------------------------------
// Expectations parsing
// ---------------------------------------------------------------------

/// Accumulates one `[[expect]]` block's `key = "value"` lines before
/// [`ExpectBuilder::finish`] validates it into an [`Expectation`].
#[derive(Default)]
struct ExpectBuilder {
    cell: Option<String>,
    profile: Option<String>,
    verdict: Option<String>,
    reason: Option<String>,
    reference: Option<String>,
    failure_contains: Option<String>,
}

impl ExpectBuilder {
    fn set(&mut self, key: &str, value: String, line_no: usize) -> Result<(), String> {
        match key {
            "cell" => {
                validate_cell_pattern(&value, line_no)?;
                self.cell = Some(value);
            }
            "profile" => self.profile = Some(value),
            "verdict" => self.verdict = Some(value),
            "reason" => self.reason = Some(value),
            "ref" => self.reference = Some(value),
            "failure_contains" => self.failure_contains = Some(value),
            other => {
                return Err(format!(
                    "expectations line {line_no}: unknown key `{other}`"
                ));
            }
        }
        Ok(())
    }

    /// Validate the accumulated block, reporting missing-required-key
    /// errors against `end_line` (the line the block closed at — either
    /// the next `[[expect]]` or end of file).
    fn finish(self, end_line: usize) -> Result<Expectation, String> {
        let cell = self.cell.ok_or_else(|| {
            format!("expectations block ending at line {end_line}: missing required key `cell`")
        })?;
        let profile = self.profile.ok_or_else(|| {
            format!("expectations block ending at line {end_line}: missing required key `profile`")
        })?;
        let verdict_str = self.verdict.ok_or_else(|| {
            format!("expectations block ending at line {end_line}: missing required key `verdict`")
        })?;
        let verdict = match verdict_str.as_str() {
            "expected_unsupported" => ExpectVerdict::ExpectedUnsupported,
            "known_flaky" => ExpectVerdict::KnownFlaky,
            other => {
                return Err(format!(
                    "expectations block ending at line {end_line}: unknown verdict `{other}` \
                     (want `expected_unsupported` or `known_flaky`)"
                ));
            }
        };
        let reason = self.reason.ok_or_else(|| {
            format!("expectations block ending at line {end_line}: missing required key `reason`")
        })?;
        Ok(Expectation {
            cell,
            profile,
            verdict,
            reason,
            reference: self.reference,
            failure_contains: self.failure_contains,
        })
    }
}

/// A `cell` pattern may only use `*` as its final character (a trailing
/// prefix-match glob). Reject it anywhere else, at parse time, so a typo
/// like `decode/*/foo` fails loudly here rather than silently matching
/// nothing (or everything) at merge time.
fn validate_cell_pattern(value: &str, line_no: usize) -> Result<(), String> {
    if let Some(pos) = value.find('*') {
        if pos != value.len() - 1 {
            return Err(format!(
                "expectations line {line_no}: `*` only allowed as the final character of \
                 `cell`, got: {value:?}"
            ));
        }
    }
    Ok(())
}

/// Parse one non-blank, non-comment line as `key = "value"`. Keys are
/// bare lowercase/underscore identifiers; values must be exactly one
/// quoted string with no embedded `"` — nothing else on the line.
fn parse_kv_line(line: &str, line_no: usize) -> Result<(String, String), String> {
    let (key, rest) = line.split_once('=').ok_or_else(|| {
        format!("expectations line {line_no}: expected `key = \"value\"`, got: {line:?}")
    })?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        return Err(format!("expectations line {line_no}: invalid key {key:?}"));
    }
    let rest = rest.trim();
    if rest.len() < 2 || !rest.starts_with('"') || !rest.ends_with('"') {
        return Err(format!(
            "expectations line {line_no}: value must be a quoted string, got: {rest:?}"
        ));
    }
    let inner = &rest[1..rest.len() - 1];
    if inner.contains('"') {
        return Err(format!(
            "expectations line {line_no}: unexpected `\"` inside value: {rest:?}"
        ));
    }
    Ok((key.to_string(), inner.to_string()))
}

/// Parse the expectations file format: `#`-comment lines, blank lines,
/// `[[expect]]` block markers, and `key = "value"` lines within a block.
/// Anything else is a loud parse error naming the offending line number.
pub fn parse_expectations(text: &str) -> Result<Vec<Expectation>, String> {
    let mut out = Vec::new();
    let mut current: Option<ExpectBuilder> = None;
    let mut last_line = 0;

    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = idx + 1;
        last_line = line_no;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[expect]]" {
            if let Some(builder) = current.take() {
                out.push(builder.finish(line_no)?);
            }
            current = Some(ExpectBuilder::default());
            continue;
        }
        let (key, value) = parse_kv_line(line, line_no)?;
        match current.as_mut() {
            Some(builder) => builder.set(&key, value, line_no)?,
            None => {
                return Err(format!(
                    "expectations line {line_no}: `{key} = ...` outside of an [[expect]] block"
                ));
            }
        }
    }
    if let Some(builder) = current.take() {
        out.push(builder.finish(last_line)?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// Matching + merge
// ---------------------------------------------------------------------

/// Does `pattern` (an [`Expectation::cell`], already validated by
/// [`validate_cell_pattern`]) match cell id `id`? A trailing `*` is a
/// prefix match; anything else is a literal match.
fn cell_pattern_matches(pattern: &str, id: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => id.starts_with(prefix),
        None => pattern == id,
    }
}

/// First expectation (in file order) whose `cell` pattern and exact
/// `profile` both match. Ambiguous multi-match expectations files are
/// the expectations author's problem, not merge's — file order wins.
///
/// `failure_text` distinguishes the two call sites in [`build_results`]:
/// - `Some(joined_failures)` (the `FAIL`-matching path): an expectation
///   with `failure_contains` set only matches if `failure_text` contains
///   that substring — a `FAIL` on the same `(cell, profile)` whose text
///   does *not* contain it is skipped in favor of a later candidate (or,
///   if none match, surfaces as an unexpected `FAIL`, never silently
///   absorbed by a same-cell expectation written for a *different*
///   failure mode).
/// - `None` (the `PASS`-staleness-check path): `failure_contains` is
///   never applied — staleness is a property of the whole `(cell,
///   profile)` pair, independent of which specific failure text the
///   expectation was originally written against.
///
/// An expectation with no `failure_contains` matches either way, exactly
/// as before this key existed.
fn find_expectation<'a>(
    expectations: &'a [Expectation],
    cell_id: &str,
    cell_profile: &str,
    failure_text: Option<&str>,
) -> Option<&'a Expectation> {
    expectations.iter().find(|e| {
        e.profile == cell_profile
            && cell_pattern_matches(&e.cell, cell_id)
            && match (&e.failure_contains, failure_text) {
                (Some(substr), Some(text)) => text.contains(substr.as_str()),
                (Some(_), None) | (None, _) => true,
            }
    })
}

/// Apply `expectations` to `raw_cells`, producing the merged, tallied
/// [`Results`]. Pure (no I/O) — [`merge`] is the file-driven wrapper
/// around this.
pub fn build_results(
    raw_cells: Vec<RawCell>,
    expectations: &[Expectation],
    meta: serde_json::Value,
) -> Results {
    let mut cells = Vec::with_capacity(raw_cells.len());
    let mut stale = Vec::new();

    for raw in raw_cells {
        let (verdict, known_flaky, expectation_reason, expectation_ref) = match raw.verdict {
            RawVerdict::SkippedToolMissing => (Verdict::SkippedToolMissing, false, None, None),
            RawVerdict::Pass => {
                // Staleness is checked by (cell, profile) alone —
                // `failure_contains` never applies here (see
                // find_expectation's doc comment).
                let matched = find_expectation(expectations, &raw.id, &raw.profile, None);
                if let Some(exp) = matched {
                    if exp.verdict == ExpectVerdict::ExpectedUnsupported {
                        stale.push(StaleExpectation {
                            cell: exp.cell.clone(),
                            profile: exp.profile.clone(),
                            reason: exp.reason.clone(),
                        });
                    }
                    // A `known_flaky` expectation matching a PASS is
                    // normal (see the module + type docs) — no stale
                    // entry either way.
                }
                (Verdict::Pass, false, None, None)
            }
            RawVerdict::Fail => {
                let failure_text = raw.failures.join("; ");
                let matched =
                    find_expectation(expectations, &raw.id, &raw.profile, Some(&failure_text));
                match matched {
                    Some(exp) => (
                        Verdict::ExpectedUnsupported,
                        exp.verdict == ExpectVerdict::KnownFlaky,
                        Some(exp.reason.clone()),
                        exp.reference.clone(),
                    ),
                    None => (Verdict::Fail, false, None, None),
                }
            }
        };

        cells.push(MergedCell {
            id: raw.id,
            profile: raw.profile,
            peer: raw.peer,
            direction: raw.direction,
            tier: raw.tier,
            verdict,
            known_flaky,
            failures: raw.failures,
            metrics: raw.metrics,
            log: raw.log,
            expectation_reason,
            expectation_ref,
        });
    }

    let summary = Summary {
        total: cells.len(),
        pass: cells.iter().filter(|c| c.verdict == Verdict::Pass).count(),
        fail: cells.iter().filter(|c| c.verdict == Verdict::Fail).count(),
        expected_unsupported: cells
            .iter()
            .filter(|c| c.verdict == Verdict::ExpectedUnsupported)
            .count(),
        skipped_tool_missing: cells
            .iter()
            .filter(|c| c.verdict == Verdict::SkippedToolMissing)
            .count(),
        stale_expectations: stale,
    };

    Results {
        meta,
        cells,
        summary,
    }
}

/// Read every `*.json` file in `dir` (sorted by filename, for
/// deterministic output) and parse each as a [`RawCell`].
fn read_cells_dir(dir: &Path) -> Result<Vec<RawCell>, String> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("read_dir {}: {e}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    paths.sort();

    let mut cells = Vec::with_capacity(paths.len());
    for path in &paths {
        let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let cell: RawCell =
            serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
        cells.push(cell);
    }
    Ok(cells)
}

/// `report merge --cells-dir DIR --expectations FILE --meta FILE --out
/// results.json`: read every per-cell JSON file in `cells_dir`, apply
/// `expectations_path`'s expectations, embed `meta_path`'s contents
/// verbatim, and write the result to `out_path`. Returns the same
/// [`Results`] that was written — the caller decides the process exit
/// code from `results.summary.fail` (see the module doc).
///
/// Errors (rather than returning an empty, trivially-all-PASS
/// [`Results`]) if `cells_dir` contains zero `*.json` files — the same
/// "never silently absorbed" property the module doc describes for
/// individual failures also has to hold for the degenerate case of an
/// orchestrator that crashed before writing any cell at all, or a
/// `--cells-dir` typo.
pub fn merge(
    cells_dir: &Path,
    expectations_path: &Path,
    meta_path: &Path,
    out_path: &Path,
) -> Result<Results, String> {
    let expectations_text = fs::read_to_string(expectations_path)
        .map_err(|e| format!("read {}: {e}", expectations_path.display()))?;
    let expectations = parse_expectations(&expectations_text)
        .map_err(|e| format!("{}: {e}", expectations_path.display()))?;

    let meta_text =
        fs::read_to_string(meta_path).map_err(|e| format!("read {}: {e}", meta_path.display()))?;
    let meta: serde_json::Value = serde_json::from_str(&meta_text)
        .map_err(|e| format!("parse {}: {e}", meta_path.display()))?;

    let raw_cells = read_cells_dir(cells_dir)?;
    if raw_cells.is_empty() {
        return Err(format!(
            "no *.json cell files found in {} — refusing to write a clean all-PASS report for \
             zero cells (an empty --cells-dir most likely means the orchestrator crashed \
             before writing any cell, or the path is wrong)",
            cells_dir.display()
        ));
    }
    let results = build_results(raw_cells, &expectations, meta);

    let json = serde_json::to_string_pretty(&results).expect("Results always serializes");
    fs::write(out_path, json).map_err(|e| format!("write {}: {e}", out_path.display()))?;

    Ok(results)
}

// ---------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------

/// Verdict cell text and notes-column text for one [`MergedCell`].
fn render_verdict(cell: &MergedCell) -> (&'static str, String) {
    match cell.verdict {
        Verdict::Pass => ("✅ PASS", String::new()),
        Verdict::Fail => ("❌ FAIL", cell.failures.join("; ")),
        Verdict::ExpectedUnsupported => {
            let label = if cell.known_flaky {
                "⚠️ KNOWN-FLAKY"
            } else {
                "⚠️ EXPECTED-UNSUPPORTED"
            };
            let mut notes = cell.expectation_reason.clone().unwrap_or_default();
            if let Some(r) = &cell.expectation_ref {
                notes.push_str(&format!(" ({r})"));
            }
            (label, notes)
        }
        Verdict::SkippedToolMissing => ("⏭ SKIPPED", cell.failures.join("; ")),
    }
}

/// Render `results` as the published markdown evidence page: a `**Meta**`
/// header (sorted `results.meta` keys), a `**Summary**` counts block, one
/// verdict table per axis (the cell id's segment before its first `/`,
/// axes in alphabetical order; a group with more than
/// `AXIS_DETAILS_THRESHOLD` rows collapses into a `<details>` block),
/// and — only if nonempty — a `**Stale expectations**` warnings section.
pub fn render_markdown(results: &Results) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    writeln!(out, "# Interop Report").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "**Meta**").unwrap();
    writeln!(out).unwrap();
    if let serde_json::Value::Object(map) = &results.meta {
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for k in keys {
            let rendered = match &map[k] {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            writeln!(out, "- {k}: {rendered}").unwrap();
        }
    }
    writeln!(out).unwrap();

    writeln!(out, "**Summary**").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "- Total: {}", results.summary.total).unwrap();
    writeln!(out, "- PASS: {}", results.summary.pass).unwrap();
    writeln!(out, "- FAIL: {}", results.summary.fail).unwrap();
    writeln!(
        out,
        "- EXPECTED-UNSUPPORTED: {}",
        results.summary.expected_unsupported
    )
    .unwrap();
    writeln!(out, "- SKIPPED: {}", results.summary.skipped_tool_missing).unwrap();
    writeln!(out).unwrap();

    let mut axes: BTreeMap<&str, Vec<&MergedCell>> = BTreeMap::new();
    for cell in &results.cells {
        let axis = cell
            .id
            .split('/')
            .next()
            .expect("str::split always yields at least one item");
        axes.entry(axis).or_default().push(cell);
    }

    for (axis, rows) in &axes {
        let wrap = rows.len() > AXIS_DETAILS_THRESHOLD;
        if wrap {
            writeln!(
                out,
                "<details><summary>{axis} ({} rows)</summary>",
                rows.len()
            )
            .unwrap();
            writeln!(out).unwrap();
        } else {
            writeln!(out, "## {axis}").unwrap();
            writeln!(out).unwrap();
        }
        writeln!(
            out,
            "| Cell | Profile | Peer | Direction | Tier | Verdict | Notes |"
        )
        .unwrap();
        writeln!(out, "| --- | --- | --- | --- | --- | --- | --- |").unwrap();
        for cell in rows {
            let (label, notes) = render_verdict(cell);
            writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} | {} |",
                cell.id, cell.profile, cell.peer, cell.direction, cell.tier, label, notes
            )
            .unwrap();
        }
        writeln!(out).unwrap();
        if wrap {
            writeln!(out, "</details>").unwrap();
            writeln!(out).unwrap();
        }
    }

    if !results.summary.stale_expectations.is_empty() {
        writeln!(out, "**Stale expectations**").unwrap();
        writeln!(out).unwrap();
        for s in &results.summary.stale_expectations {
            writeln!(
                out,
                "- `{}` (profile `{}`): {} — matched cell now PASSes; consider removing.",
                s.cell, s.profile, s.reason
            )
            .unwrap();
        }
    }

    out
}

/// `report render --in results.json --out results.md`: read a
/// [`Results`] JSON document from `in_path`, render it via
/// [`render_markdown`], and write the markdown to `out_path`. Returns
/// the rendered markdown (the `--github-summary` CLI flag reuses it via
/// [`append_github_summary`] without a re-read).
pub fn render(in_path: &Path, out_path: &Path) -> Result<String, String> {
    let text =
        fs::read_to_string(in_path).map_err(|e| format!("read {}: {e}", in_path.display()))?;
    let results: Results =
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", in_path.display()))?;
    let md = render_markdown(&results);
    fs::write(out_path, &md).map_err(|e| format!("write {}: {e}", out_path.display()))?;
    Ok(md)
}

/// Append `markdown` to the file at `path`, creating it if it doesn't
/// exist yet. Split out from [`render`] so `--github-summary`'s
/// `GITHUB_STEP_SUMMARY` env-var lookup (which this module does not do
/// itself, keeping it testable without touching real process state)
/// stays in the CLI layer, while the append behavior itself stays
/// testable here against an ordinary temp file.
pub fn append_github_summary(path: &Path, markdown: &str) -> Result<(), String> {
    use std::io::Write as _;

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    file.write_all(markdown.as_bytes())
        .map_err(|e| format!("append {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_cell(id: &str, profile: &str, verdict: RawVerdict) -> RawCell {
        RawCell {
            id: id.to_string(),
            profile: profile.to_string(),
            peer: "ffmpeg".to_string(),
            direction: "recv".to_string(),
            tier: "remux".to_string(),
            verdict,
            failures: Vec::new(),
            metrics: None,
            log: format!("{id}.log"),
        }
    }

    fn expectation(cell: &str, profile: &str, verdict: ExpectVerdict, reason: &str) -> Expectation {
        Expectation {
            cell: cell.to_string(),
            profile: profile.to_string(),
            verdict,
            reason: reason.to_string(),
            reference: None,
            failure_contains: None,
        }
    }

    fn raw_cell_with_failures(
        id: &str,
        profile: &str,
        verdict: RawVerdict,
        failures: &[&str],
    ) -> RawCell {
        RawCell {
            failures: failures.iter().map(|s| s.to_string()).collect(),
            ..raw_cell(id, profile, verdict)
        }
    }

    fn expectation_with_failure_contains(
        cell: &str,
        profile: &str,
        verdict: ExpectVerdict,
        reason: &str,
        failure_contains: &str,
    ) -> Expectation {
        Expectation {
            failure_contains: Some(failure_contains.to_string()),
            ..expectation(cell, profile, verdict, reason)
        }
    }

    // (a) all-PASS cells -> exit-0 summary (summary.fail == 0).
    #[test]
    fn all_pass_cells_yield_zero_fail() {
        let raw = vec![
            raw_cell("decode/ffmpeg", "baseline", RawVerdict::Pass),
            raw_cell("decode/mpv", "baseline", RawVerdict::Pass),
        ];
        let results = build_results(raw, &[], serde_json::json!({}));

        assert_eq!(results.summary.fail, 0);
        assert_eq!(results.summary.pass, 2);
        assert_eq!(results.summary.total, 2);
        assert!(results.summary.stale_expectations.is_empty());
    }

    // (b) one unexpected FAIL -> nonzero + named (findable by id in the
    // merged cells with verdict Fail).
    #[test]
    fn unexpected_fail_is_nonzero_and_named() {
        let raw = vec![
            raw_cell("decode/ffmpeg", "baseline", RawVerdict::Pass),
            raw_cell("srt/us-to-them", "baseline", RawVerdict::Fail),
        ];
        let results = build_results(raw, &[], serde_json::json!({}));

        assert_eq!(results.summary.fail, 1);
        let failed = results
            .cells
            .iter()
            .find(|c| c.id == "srt/us-to-them")
            .expect("failed cell must be present in the merged output");
        assert_eq!(failed.verdict, Verdict::Fail);
    }

    // (c) FAIL matched by expectation -> run passes, cell verdict
    // EXPECTED_UNSUPPORTED.
    #[test]
    fn matched_fail_becomes_expected_unsupported() {
        let raw = vec![raw_cell("decode/mpv", "baseline", RawVerdict::Fail)];
        let exp = expectation(
            "decode/mpv",
            "baseline",
            ExpectVerdict::ExpectedUnsupported,
            "mpv lacks async KLV support",
        );
        let results = build_results(raw, &[exp], serde_json::json!({}));

        assert_eq!(results.summary.fail, 0);
        assert_eq!(results.summary.expected_unsupported, 1);
        assert_eq!(results.cells[0].verdict, Verdict::ExpectedUnsupported);
        assert!(!results.cells[0].known_flaky);
        assert_eq!(
            results.cells[0].expectation_reason.as_deref(),
            Some("mpv lacks async KLV support")
        );
    }

    // --- failure_contains ---

    // A FAIL whose joined failures text contains the expectation's
    // `failure_contains` substring matches, exactly like a plain
    // (cell, profile)-only expectation would.
    #[test]
    fn failure_contains_matches_when_substring_present() {
        let raw = vec![raw_cell_with_failures(
            "srt-live/tsp-to-us",
            "baseline",
            RawVerdict::Fail,
            &["byte-transparent tier: stream_sha256 mismatch (source abc, received def)"],
        )];
        let exp = expectation_with_failure_contains(
            "srt-live/tsp-to-us",
            "baseline",
            ExpectVerdict::ExpectedUnsupported,
            "SRT-specific tail loss",
            "stream_sha256 mismatch",
        );
        let results = build_results(raw, &[exp], serde_json::json!({}));

        assert_eq!(results.summary.fail, 0);
        assert_eq!(results.summary.expected_unsupported, 1);
        assert_eq!(results.cells[0].verdict, Verdict::ExpectedUnsupported);
    }

    // A FAIL on the same (cell, profile) but a DIFFERENT failure mode
    // (text doesn't contain the substring) must NOT be silently
    // absorbed — this is the whole point of the key: mechanism-blind
    // (cell, profile)-only matching would have hidden this as
    // ExpectedUnsupported; failure_contains keeps it a real FAIL.
    #[test]
    fn failure_contains_non_match_stays_fail() {
        let raw = vec![raw_cell_with_failures(
            "srt-live/tsp-to-us",
            "baseline",
            RawVerdict::Fail,
            &["recv FAILed: video AUs: got 0, want >= 168"],
        )];
        let exp = expectation_with_failure_contains(
            "srt-live/tsp-to-us",
            "baseline",
            ExpectVerdict::ExpectedUnsupported,
            "SRT-specific tail loss",
            "stream_sha256 mismatch",
        );
        let results = build_results(raw, &[exp], serde_json::json!({}));

        assert_eq!(results.summary.fail, 1);
        assert_eq!(results.summary.expected_unsupported, 0);
        assert_eq!(results.cells[0].verdict, Verdict::Fail);
    }

    // An expectation with no `failure_contains` key set (the common
    // case, and every pre-existing row) matches a FAIL regardless of
    // its failure text — unchanged behavior from before this key
    // existed.
    #[test]
    fn absent_failure_contains_matches_any_failure_text() {
        let raw = vec![raw_cell_with_failures(
            "decode/mpv",
            "baseline",
            RawVerdict::Fail,
            &["anything at all, unrelated to the expectation's own reason text"],
        )];
        let exp = expectation(
            "decode/mpv",
            "baseline",
            ExpectVerdict::ExpectedUnsupported,
            "mpv lacks async KLV support",
        );
        let results = build_results(raw, &[exp], serde_json::json!({}));

        assert_eq!(results.summary.fail, 0);
        assert_eq!(results.cells[0].verdict, Verdict::ExpectedUnsupported);
    }

    // A `failure_contains` expectation must still catch staleness when
    // its cell PASSes — the substring constraint only narrows FAIL
    // matching, never a PASS staleness check (there's no failure text
    // to test the substring against on a PASS in the first place).
    #[test]
    fn failure_contains_expectation_is_still_stale_on_pass() {
        let raw = vec![raw_cell("srt-live/tsp-to-us", "baseline", RawVerdict::Pass)];
        let exp = expectation_with_failure_contains(
            "srt-live/tsp-to-us",
            "baseline",
            ExpectVerdict::ExpectedUnsupported,
            "SRT-specific tail loss",
            "stream_sha256 mismatch",
        );
        let results = build_results(raw, &[exp], serde_json::json!({}));

        assert_eq!(results.summary.stale_expectations.len(), 1);
        assert_eq!(results.cells[0].verdict, Verdict::Pass);
    }

    // Two candidates for the SAME (cell, profile): the first has a
    // `failure_contains` that does NOT match this FAIL's text, the
    // second is unconstrained. `find_expectation`'s single-predicate
    // `Iterator::find` must skip the non-matching first candidate and
    // resolve via the second, rather than stopping at (and shadowing
    // behind) the first just because cell+profile matched — pins this
    // against a future two-phase (match cell+profile first, filter by
    // failure_contains second) refactor that could silently break it.
    #[test]
    fn non_matching_failure_contains_falls_through_to_next_candidate() {
        let raw = vec![raw_cell_with_failures(
            "srt-live/ffmpeg-to-us",
            "baseline",
            RawVerdict::Fail,
            &["recv FAILed: KLV records: got 0, want >= 56 (10 Hz x 8s x 70% slack)"],
        )];
        let decoy = expectation_with_failure_contains(
            "srt-live/ffmpeg-to-us",
            "baseline",
            ExpectVerdict::ExpectedUnsupported,
            "wrong mechanism — should never be selected",
            "stream_sha256 mismatch",
        );
        let real = expectation(
            "srt-live/ffmpeg-to-us",
            "baseline",
            ExpectVerdict::ExpectedUnsupported,
            "KLV payload truncation",
        );
        let results = build_results(raw, &[decoy, real], serde_json::json!({}));

        assert_eq!(results.summary.fail, 0);
        assert_eq!(results.cells[0].verdict, Verdict::ExpectedUnsupported);
        assert_eq!(
            results.cells[0].expectation_reason.as_deref(),
            Some("KLV payload truncation"),
            "must resolve via the second (matching) candidate, not the first (non-matching, cell+profile-only) one"
        );
    }

    // (d) expectation whose cell PASSes -> stale_expectations entry.
    #[test]
    fn passing_cell_with_expected_unsupported_expectation_is_stale() {
        let raw = vec![raw_cell("decode/mpv", "baseline", RawVerdict::Pass)];
        let exp = expectation(
            "decode/mpv",
            "baseline",
            ExpectVerdict::ExpectedUnsupported,
            "mpv lacks async KLV support",
        );
        let results = build_results(raw, &[exp], serde_json::json!({}));

        assert_eq!(results.summary.stale_expectations.len(), 1);
        assert_eq!(results.summary.stale_expectations[0].cell, "decode/mpv");
        assert_eq!(results.cells[0].verdict, Verdict::Pass);
    }

    // (e) glob decode/* matches decode/mpv.
    #[test]
    fn glob_pattern_matches_prefix() {
        let raw = vec![raw_cell("decode/mpv", "baseline", RawVerdict::Fail)];
        let exp = expectation(
            "decode/*",
            "baseline",
            ExpectVerdict::ExpectedUnsupported,
            "decode gap",
        );
        let results = build_results(raw, &[exp], serde_json::json!({}));

        assert_eq!(results.cells[0].verdict, Verdict::ExpectedUnsupported);
        assert_eq!(results.summary.fail, 0);
    }

    // (f) SKIPPED_TOOL_MISSING counted, never fatal, never stale-matched
    // (even when a matching expectation exists).
    #[test]
    fn skipped_tool_missing_is_never_fatal_or_stale() {
        let raw = vec![raw_cell(
            "decode/mpv",
            "baseline",
            RawVerdict::SkippedToolMissing,
        )];
        let exp = expectation(
            "decode/mpv",
            "baseline",
            ExpectVerdict::ExpectedUnsupported,
            "would-be-stale reason",
        );
        let results = build_results(raw, &[exp], serde_json::json!({}));

        assert_eq!(results.summary.fail, 0);
        assert_eq!(results.summary.skipped_tool_missing, 1);
        assert_eq!(results.cells[0].verdict, Verdict::SkippedToolMissing);
        assert!(results.summary.stale_expectations.is_empty());
    }

    #[test]
    fn known_flaky_fail_is_expected_unsupported_and_flagged() {
        let raw = vec![raw_cell("srt/flaky", "baseline", RawVerdict::Fail)];
        let exp = expectation(
            "srt/flaky",
            "baseline",
            ExpectVerdict::KnownFlaky,
            "intermittent timeout, see TICKET-9",
        );
        let results = build_results(raw, &[exp], serde_json::json!({}));

        assert_eq!(results.summary.fail, 0);
        assert_eq!(results.cells[0].verdict, Verdict::ExpectedUnsupported);
        assert!(results.cells[0].known_flaky);
    }

    #[test]
    fn known_flaky_pass_is_normal_never_stale() {
        let raw = vec![raw_cell("srt/flaky", "baseline", RawVerdict::Pass)];
        let exp = expectation(
            "srt/flaky",
            "baseline",
            ExpectVerdict::KnownFlaky,
            "intermittent timeout, see TICKET-9",
        );
        let results = build_results(raw, &[exp], serde_json::json!({}));

        assert_eq!(results.cells[0].verdict, Verdict::Pass);
        assert!(
            results.summary.stale_expectations.is_empty(),
            "a known_flaky expectation matching a PASS must never be reported stale"
        );
    }

    #[test]
    fn profile_mismatch_does_not_match() {
        let raw = vec![raw_cell("decode/mpv", "hevc", RawVerdict::Fail)];
        let exp = expectation(
            "decode/mpv",
            "baseline",
            ExpectVerdict::ExpectedUnsupported,
            "wrong profile",
        );
        let results = build_results(raw, &[exp], serde_json::json!({}));

        assert_eq!(results.cells[0].verdict, Verdict::Fail);
        assert_eq!(results.summary.fail, 1);
    }

    #[test]
    fn cell_pattern_matches_exact_and_trailing_glob() {
        assert!(cell_pattern_matches("decode/mpv", "decode/mpv"));
        assert!(!cell_pattern_matches("decode/mpv", "decode/mpv2"));
        assert!(cell_pattern_matches("decode/*", "decode/mpv"));
        assert!(cell_pattern_matches("decode/*", "decode/"));
        assert!(!cell_pattern_matches("decode/*", "encode/mpv"));
    }

    // (g) render golden: small fixed Results -> exact expected markdown.
    #[test]
    fn render_golden() {
        let results = Results {
            meta: serde_json::json!({"host": "ci-runner-1", "profile_seconds": 30}),
            cells: vec![
                MergedCell {
                    id: "decode/ffmpeg".to_string(),
                    profile: "baseline".to_string(),
                    peer: "ffmpeg".to_string(),
                    direction: "recv".to_string(),
                    tier: "remux".to_string(),
                    verdict: Verdict::Pass,
                    known_flaky: false,
                    failures: Vec::new(),
                    metrics: None,
                    log: "decode-ffmpeg.log".to_string(),
                    expectation_reason: None,
                    expectation_ref: None,
                },
                MergedCell {
                    id: "decode/mpv".to_string(),
                    profile: "baseline".to_string(),
                    peer: "mpv".to_string(),
                    direction: "recv".to_string(),
                    tier: "remux".to_string(),
                    verdict: Verdict::ExpectedUnsupported,
                    known_flaky: false,
                    failures: Vec::new(),
                    metrics: None,
                    log: "decode-mpv.log".to_string(),
                    expectation_reason: Some("mpv lacks async KLV support".to_string()),
                    expectation_ref: Some("TICKET-123".to_string()),
                },
                MergedCell {
                    id: "srt/us-to-ffmpeg".to_string(),
                    profile: "baseline".to_string(),
                    peer: "ffmpeg".to_string(),
                    direction: "send".to_string(),
                    tier: "wire".to_string(),
                    verdict: Verdict::Fail,
                    known_flaky: false,
                    failures: vec!["timeout waiting for connect".to_string()],
                    metrics: None,
                    log: "srt-us-to-ffmpeg.log".to_string(),
                    expectation_reason: None,
                    expectation_ref: None,
                },
            ],
            summary: Summary {
                total: 3,
                pass: 1,
                fail: 1,
                expected_unsupported: 1,
                skipped_tool_missing: 0,
                stale_expectations: vec![StaleExpectation {
                    cell: "decode/old".to_string(),
                    profile: "baseline".to_string(),
                    reason: "historic gap, now fixed".to_string(),
                }],
            },
        };

        let golden = "\
# Interop Report

**Meta**

- host: ci-runner-1
- profile_seconds: 30

**Summary**

- Total: 3
- PASS: 1
- FAIL: 1
- EXPECTED-UNSUPPORTED: 1
- SKIPPED: 0

## decode

| Cell | Profile | Peer | Direction | Tier | Verdict | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| decode/ffmpeg | baseline | ffmpeg | recv | remux | ✅ PASS |  |
| decode/mpv | baseline | mpv | recv | remux | ⚠️ EXPECTED-UNSUPPORTED | mpv lacks async KLV support (TICKET-123) |

## srt

| Cell | Profile | Peer | Direction | Tier | Verdict | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| srt/us-to-ffmpeg | baseline | ffmpeg | send | wire | ❌ FAIL | timeout waiting for connect |

**Stale expectations**

- `decode/old` (profile `baseline`): historic gap, now fixed — matched cell now PASSes; consider removing.
";

        let rendered = render_markdown(&results);
        assert_eq!(rendered, golden, "rendered markdown:\n{rendered}");
    }

    #[test]
    fn axis_group_wraps_in_details_beyond_threshold() {
        let mut cells = Vec::new();
        for i in 0..(AXIS_DETAILS_THRESHOLD + 1) {
            cells.push(MergedCell {
                id: format!("decode/tool{i}"),
                profile: "baseline".to_string(),
                peer: format!("tool{i}"),
                direction: "recv".to_string(),
                tier: "remux".to_string(),
                verdict: Verdict::Pass,
                known_flaky: false,
                failures: Vec::new(),
                metrics: None,
                log: format!("tool{i}.log"),
                expectation_reason: None,
                expectation_ref: None,
            });
        }
        let results = Results {
            meta: serde_json::json!({}),
            summary: Summary {
                total: cells.len(),
                pass: cells.len(),
                fail: 0,
                expected_unsupported: 0,
                skipped_tool_missing: 0,
                stale_expectations: Vec::new(),
            },
            cells,
        };

        let rendered = render_markdown(&results);
        assert!(
            rendered.contains("<details><summary>decode (16 rows)</summary>"),
            "expected a <details> wrapper for a 16-row group:\n{rendered}"
        );
        assert!(rendered.contains("</details>"));
    }

    #[test]
    fn axis_group_at_threshold_does_not_wrap() {
        let mut cells = Vec::new();
        for i in 0..AXIS_DETAILS_THRESHOLD {
            cells.push(MergedCell {
                id: format!("decode/tool{i}"),
                profile: "baseline".to_string(),
                peer: format!("tool{i}"),
                direction: "recv".to_string(),
                tier: "remux".to_string(),
                verdict: Verdict::Pass,
                known_flaky: false,
                failures: Vec::new(),
                metrics: None,
                log: format!("tool{i}.log"),
                expectation_reason: None,
                expectation_ref: None,
            });
        }
        let results = Results {
            meta: serde_json::json!({}),
            summary: Summary {
                total: cells.len(),
                pass: cells.len(),
                fail: 0,
                expected_unsupported: 0,
                skipped_tool_missing: 0,
                stale_expectations: Vec::new(),
            },
            cells,
        };

        let rendered = render_markdown(&results);
        assert!(!rendered.contains("<details>"));
        assert!(rendered.contains("## decode"));
    }

    // --- Expectations TOML parser ---

    #[test]
    fn parses_a_minimal_valid_expectations_file() {
        let text = "\
# a comment
[[expect]]
cell = \"decode/*\"
profile = \"baseline\"
verdict = \"expected_unsupported\"
reason = \"mpv lacks async KLV support\"
ref = \"TICKET-123\"

[[expect]]
cell = \"srt/flaky\"
profile = \"baseline\"
verdict = \"known_flaky\"
reason = \"intermittent timeout\"
";
        let parsed = parse_expectations(text).expect("valid file must parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].cell, "decode/*");
        assert_eq!(parsed[0].verdict, ExpectVerdict::ExpectedUnsupported);
        assert_eq!(parsed[0].reference.as_deref(), Some("TICKET-123"));
        assert_eq!(parsed[1].verdict, ExpectVerdict::KnownFlaky);
        assert_eq!(parsed[1].reference, None);
        assert_eq!(parsed[0].failure_contains, None);
        assert_eq!(parsed[1].failure_contains, None);
    }

    #[test]
    fn parses_the_optional_failure_contains_key() {
        let text = "\
[[expect]]
cell = \"srt-live/tsp-to-us\"
profile = \"baseline\"
verdict = \"expected_unsupported\"
reason = \"SRT-specific tail loss\"
failure_contains = \"stream_sha256 mismatch\"
";
        let parsed = parse_expectations(text).expect("valid file must parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0].failure_contains.as_deref(),
            Some("stream_sha256 mismatch")
        );
    }

    #[test]
    fn empty_and_all_comment_file_parses_to_no_expectations() {
        let text = "# nothing but comments\n\n# another\n";
        assert_eq!(parse_expectations(text).expect("must parse"), vec![]);
        assert_eq!(parse_expectations("").expect("must parse"), vec![]);
    }

    #[test]
    fn rejects_star_not_at_end_of_cell_pattern() {
        let text = "[[expect]]\ncell = \"decode/*/foo\"\nprofile = \"baseline\"\nverdict = \"expected_unsupported\"\nreason = \"x\"\n";
        let err = parse_expectations(text).unwrap_err();
        assert!(err.contains("line 2"), "error should name the line: {err}");
        assert!(err.contains('*'));
    }

    #[test]
    fn rejects_unquoted_value() {
        let text = "[[expect]]\ncell = decode/mpv\n";
        let err = parse_expectations(text).unwrap_err();
        assert!(err.contains("line 2"), "error should name the line: {err}");
        assert!(err.contains("quoted"));
    }

    #[test]
    fn rejects_value_with_embedded_quote() {
        let text = "[[expect]]\ncell = \"decode/\"mpv\"\"\n";
        let err = parse_expectations(text).unwrap_err();
        assert!(err.contains("line 2"), "error should name the line: {err}");
    }

    #[test]
    fn rejects_unknown_key() {
        let text = "[[expect]]\nbogus = \"x\"\n";
        let err = parse_expectations(text).unwrap_err();
        assert!(err.contains("unknown key"), "{err}");
    }

    #[test]
    fn rejects_kv_line_outside_a_block() {
        let text = "cell = \"decode/mpv\"\n";
        let err = parse_expectations(text).unwrap_err();
        assert!(err.contains("outside of an [[expect]] block"), "{err}");
    }

    #[test]
    fn rejects_missing_required_key() {
        let text = "[[expect]]\ncell = \"decode/mpv\"\nprofile = \"baseline\"\nverdict = \"expected_unsupported\"\n";
        let err = parse_expectations(text).unwrap_err();
        assert!(err.contains("missing required key `reason`"), "{err}");
    }

    #[test]
    fn rejects_unknown_verdict_value() {
        let text = "[[expect]]\ncell = \"decode/mpv\"\nprofile = \"baseline\"\nverdict = \"bogus\"\nreason = \"x\"\n";
        let err = parse_expectations(text).unwrap_err();
        assert!(err.contains("unknown verdict"), "{err}");
    }

    #[test]
    fn rejects_garbage_line() {
        let text = "[[expect]]\nnot a key value line at all\n";
        let err = parse_expectations(text).unwrap_err();
        assert!(err.contains("line 2"), "{err}");
    }

    // --- File-driven merge/render round trip ---

    #[test]
    fn merge_and_render_end_to_end_via_tempdir() {
        let dir = std::env::temp_dir().join(format!(
            "tst-interop-report-merge-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time moves forward")
                .as_nanos()
        ));
        let cells_dir = dir.join("cells");
        fs::create_dir_all(&cells_dir).expect("create cells dir");

        fs::write(
            cells_dir.join("a.json"),
            serde_json::to_string(&raw_cell("decode/ffmpeg", "baseline", RawVerdict::Pass))
                .unwrap(),
        )
        .expect("write cell a");
        fs::write(
            cells_dir.join("b.json"),
            serde_json::to_string(&raw_cell("decode/mpv", "baseline", RawVerdict::Fail)).unwrap(),
        )
        .expect("write cell b");

        let expectations_path = dir.join("expectations.toml");
        fs::write(
            &expectations_path,
            "[[expect]]\ncell = \"decode/mpv\"\nprofile = \"baseline\"\nverdict = \"expected_unsupported\"\nreason = \"gap\"\n",
        )
        .expect("write expectations");

        let meta_path = dir.join("meta.json");
        fs::write(&meta_path, r#"{"host": "test-host"}"#).expect("write meta");

        let out_path = dir.join("results.json");
        let results = merge(&cells_dir, &expectations_path, &meta_path, &out_path)
            .expect("merge must succeed");

        assert_eq!(results.summary.fail, 0);
        assert_eq!(results.summary.pass, 1);
        assert_eq!(results.summary.expected_unsupported, 1);
        assert!(out_path.exists());

        let md_path = dir.join("results.md");
        let md = render(&out_path, &md_path).expect("render must succeed");
        assert!(md.contains("# Interop Report"));
        assert!(
            fs::read_to_string(&md_path)
                .expect("read md")
                .contains("decode/ffmpeg")
        );

        let summary_path = dir.join("summary.md");
        append_github_summary(&summary_path, &md).expect("append must succeed");
        append_github_summary(&summary_path, "\nmore\n").expect("second append must succeed");
        let appended = fs::read_to_string(&summary_path).expect("read summary");
        assert!(appended.starts_with("# Interop Report"));
        assert!(appended.trim_end().ends_with("more"));

        let _ = fs::remove_dir_all(&dir);
    }

    /// An empty `--cells-dir` must never produce a clean, all-PASS
    /// `results.json` — that's the worst failure mode for this tool (a
    /// crashed orchestrator, or a `--cells-dir` typo, silently rendering
    /// as green evidence). `merge` must error instead of returning an
    /// empty-but-successful `Results`, and must not write `--out`.
    #[test]
    fn merge_with_empty_cells_dir_is_a_hard_error() {
        let dir = std::env::temp_dir().join(format!(
            "tst-interop-report-merge-empty-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time moves forward")
                .as_nanos()
        ));
        let cells_dir = dir.join("cells");
        fs::create_dir_all(&cells_dir).expect("create empty cells dir");
        // A non-`.json` file must not be mistaken for a cell either.
        fs::write(cells_dir.join("README.txt"), "not a cell").expect("write stray file");

        let expectations_path = dir.join("expectations.toml");
        fs::write(&expectations_path, "# no expectations\n").expect("write expectations");

        let meta_path = dir.join("meta.json");
        fs::write(&meta_path, r#"{"host": "test-host"}"#).expect("write meta");

        let out_path = dir.join("results.json");
        let err = merge(&cells_dir, &expectations_path, &meta_path, &out_path)
            .expect_err("merge over zero cell files must be an error, not a clean empty report");

        assert!(
            err.contains(&cells_dir.display().to_string()),
            "error must name the empty directory: {err}"
        );
        assert!(
            !out_path.exists(),
            "merge must not write --out on this error path"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}

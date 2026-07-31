# Releasing

`ts-transformer` publishes two artifacts on a single `v*` tag: **PyPI**
(`tstrans` wheels) and **Maven Central** (`org.tstrans:tstrans-jvm`). PyPI
publishes automatically (OIDC trusted publishing); Maven uploads to the
Central Portal in a **staged** state that a maintainer releases manually.

> **Release state (updated 2026-06-23 — v0.2.0 shipped).** Both registries are
> now live at **0.2.0**: `tstrans` on PyPI (the first-ever PyPI publish — the
> OIDC trusted-publisher path ran successfully) and
> `org.tstrans:tstrans-jvm:0.2.0` on Maven Central. Future releases are normal
> *subsequent* publishes on both registries. Two standing cautions remain:
> Maven Central is immutable — **never tag a release without explicit
> maintainer confirmation** — and on the tag run **watch the OIDC
> `publish to PyPI` job**: a *skipped* publish means nothing reached PyPI (the
> way v0.1.0 silently missed).

## One-time prerequisites

1. **GPG signing key** (for Maven artifact signatures):
   ```bash
   gpg --full-generate-key            # RSA 4096, your identity
   gpg --list-secret-keys --keyid-format=long
   # Publish the public key so Central can verify signatures:
   gpg --keyserver keyserver.ubuntu.com --send-keys <KEYID>
   # Export the ASCII-armored private key for the CI secret:
   gpg --armor --export-secret-keys <KEYID> > tstrans-signing-key.asc
   ```

2. **Central Portal user token** — at https://central.sonatype.com →
   Account → Generate User Token. Yields a username + password pair.
   (The `org.tstrans` namespace is already claimed + verified.)

3. **GitHub repository secrets** (Settings → Secrets and variables → Actions):
   | Secret | Value |
   |---|---|
   | `MAVEN_CENTRAL_USERNAME` | Portal token username |
   | `MAVEN_CENTRAL_PASSWORD` | Portal token password |
   | `SIGNING_KEY` | contents of `tstrans-signing-key.asc` (full armored block, including the `-----BEGIN/END-----` lines and newlines) |
   | `SIGNING_PASSWORD` | the GPG key passphrase (empty string if none) |

   PyPI needs no secret — it uses OIDC trusted publishing, configured and
   first exercised successfully at v0.2.0.

## Pre-tag checklist

1. **Version consistency (REL-01 preflight).** The rail checks six sources —
   workspace `Cargo.toml`, `bindings/python/pyproject.toml`, the tst-py
   `bindings/python/Cargo.toml`, the C `TST_VERSION_*` constants in
   `bindings/c/core/src/lib.rs`, the committed `bindings/c/include/tstrans.h`
   macros, and the Python version test:
   ```bash
   bash scripts/check/repo/release-version-consistency.sh
   ```
   It must report all-consistent at the release version before you tag. The
   same rail runs in CI on the tag and gates both publish workflows.

   The rail does **not** cover everything the version sweep must touch —
   also update by hand:
   - the workspace `Cargo.lock`, the fuzz-workspace lockfiles
     (`crates/*/fuzz/Cargo.lock`), **and the embedded sub-project lockfiles**
     (`embedded/baremetal-qemu{,-c}/Cargo.lock`,
     `embedded/freertos-srt/example/host/Cargo.lock`) — all record workspace
     crate versions, and the embedded QEMU gates build `--locked`, so a stale
     one fails CI loudly (a `cargo metadata` run in each directory re-syncs);
   - the regenerated committed `tstrans.h` (regeneration picks up the new
     `TST_VERSION_*` values);
   - the `#define TST_VERSION_*` snippet in `docs/languages/c.md` — the
     doc-currency ratchet (`doc-abi-and-st1910-currency.sh` Rule 4) pins it
     to the workspace version and will hold CI red until it matches;
   - `crates/mbedtls-src/Cargo.toml`'s `version` field. It carries a
     `+3.6.7`-style local-version-identifier suffix pinned to the bundled
     Mbed TLS release (e.g. `0.4.0+3.6.7`) — bump the base number on every
     workspace version sweep, and bump the suffix independently whenever the
     vendored `vendor/mbedtls` submodule itself advances.
2. **`main` is green** — `ci`, `jvm-jar`, and `python-wheels` all passing on
   the commit you intend to tag.
3. **Dry-run the wheels** (recommended):
   `gh workflow run python-wheels.yml` builds every wheel + sdist and skips
   publish (publish is tag-gated). Confirm all legs are green, and watch for a
   leg sitting *queued* — that is the v0.1.0 failure mode.
4. **CHANGELOG:** retitle the `[Unreleased]` section to `[X.Y.Z] — <date>`
   and open a fresh `[Unreleased]` stub. The "Release highlights" block is
   the seed for the GitHub Release notes.

## Release procedure

(`vX.Y.Z` below stands for the release tag, e.g. `v0.3.0`.)

1. **Tag and push** (only after explicit maintainer confirmation — Maven
   Central is immutable):
   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```
2. The tag triggers both workflows:
   - **python-wheels.yml** builds wheels + sdist and **publishes to PyPI via
     OIDC trusted publishing.** **Watch the publish job:** if any required
     wheel leg is cancelled or stuck queued, the `needs:`-gated publish job
     is *skipped* (not failed); a skipped publish means nothing reached PyPI
     (the v0.1.0 failure mode).
   - **jvm-jar.yml** builds the 4 native libs, assembles the fat jar, then the
     `publish` job uploads a signed bundle to the **Central Portal (staged)**.
3. **Release Maven manually:** https://central.sonatype.com → Deployments,
   inspect the staged `org.tstrans:tstrans-jvm:X.Y.Z` bundle (validation must
   pass), then click **Publish**. It appears on Maven Central shortly after.
4. **Verify BEFORE advertising:**
   - `pip install tstrans==X.Y.Z` resolves and imports.
   - A Gradle / Maven resolve of `org.tstrans:tstrans-jvm:X.Y.Z` succeeds.
   Only **after** both verify, announce the release (GitHub Release notes
   seeded from the CHANGELOG highlights block).

## crates.io publish (Rust crates)

`ts-transformer` also publishes its 10 publishable Rust library crates to
crates.io, starting at **v0.4.0**. This is independent of the PyPI/Maven tag
trigger above — crates.io publishing is manual, per-crate, and not wired to
CI (`cargo publish` needs a maintainer-held API token; there is no
trusted-publishing / OIDC path for crates.io analogous to PyPI's yet).

**Publish order** (topo-sorted from the dependency graph — a crate cannot
publish until every crate it depends on with a `version =` key is already
live on the index):

1. `tstrans-mbedtls-src` (no internal dependencies — the base of the chain)
2. `tstrans-srt-sys`, `tstrans-rist-sys` (each depends on `tstrans-mbedtls-src`
   as a build-dependency; these two can publish in either order relative to
   each other)
3. `tst-core` (no internal path dependencies with a `version` key)
4. `tst-pipeline` (depends on `tst-core`)
5. `tst-udp`, `tst-tcp`, `tst-hls`, `tst-rtp`, `tst-srt`, `tst-rist` (each
   depends only on `tst-core` + `tst-pipeline`; publish in any order once
   layer 4 is live)

For each crate, in the order above:

```bash
cargo publish --dry-run -p <pkg>   # catches metadata/packaging problems for free
cargo publish -p <pkg>
```

**Wait for index visibility between layers** — crates.io's index takes on
the order of tens of seconds to a couple of minutes to propagate a fresh
publish; a same-session `cargo publish` for a *dependent* crate can otherwise
fail with the same "no matching package named ..." error the
`publish-package-sanity` CI rail's manual-fallback branch works around
pre-first-publish (see below). Confirm the crate's `https://crates.io/crates/<pkg>`
page is live before moving to the next layer, not just that the `cargo
publish` command returned success.

**Post-publish, before moving on:**
- Check `https://crates.io/crates/<pkg>` resolves and shows the expected
  version.
- Check the docs.rs build status at `https://docs.rs/<pkg>` — docs.rs builds
  automatically on publish; a build failure there (e.g. the vendored native
  source failing to compile in docs.rs's sandboxed build environment) is a
  real signal worth investigating even though it doesn't block the publish
  itself.

**First-publish extras (one-time, this release only):**
- Use a crates.io API token scoped to **publish-new** (not blanket
  publish-update) for the first publish of each of the 10 crates — narrower
  blast radius if the token leaks.
- **Do not** configure crates.io Trusted Publishing (OIDC) until *after* all
  10 crates exist on the index — Trusted Publishing is configured per
  already-existing crate on crates.io's side, so it cannot be set up before
  the first manual publish. Track "wire up Trusted Publishing for the 10
  crates.io crates" as a recorded post-v0.4.0 follow-up.

**Rail note:** the `publish-package-sanity` CI rail
(`scripts/check/rust/publish-package-sanity.sh`) cannot run a real `cargo
package --no-verify` on `tstrans-srt-sys` / `tstrans-rist-sys` until
`tstrans-mbedtls-src` is live on crates.io (Cargo's packaging step validates
every path-dependency-with-a-`version`-key against the real index,
independent of `--no-verify`). Until then the rail falls back to a manual
tar+gzip size reconstruction of the exact `cargo package --list` file set.
This is self-healing: once `tstrans-mbedtls-src`'s first publish lands, the
rail's real `cargo package --no-verify` calls start succeeding on their own
and the fallback branch simply stops firing — no rail change needed after
this release.

## Notes

- Maven Central releases are **permanent/immutable** — a bad bundle means
  shipping a corrective `0.2.1`, never overwriting. That is why the publish
  stages for manual inspection (`publishToMavenCentral(automaticRelease =
  false)` in `bindings/jvm/build.gradle.kts`). Confirm that flag's current
  value before publishing if you intend to change the staged behavior.
- If the Portal upload fails with an opaque error, re-run the publish with
  `--info` added to the `./gradlew publishToMavenCentral` invocation to capture
  the HTTP response body from the Portal.
- The fat jar covers linux-x86_64/aarch64, macos-aarch64, windows-x86_64.
  macOS Intel (`macos-x86_64`) is deferred — see
  [deferred-features.md](deferred-features.md). Intel-Mac Python users install
  from the sdist.

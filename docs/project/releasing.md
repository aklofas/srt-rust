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

   PyPI needs no secret — it uses OIDC trusted publishing, already configured
   (but never yet exercised — see the asymmetry note above).

## Pre-tag checklist

1. **Version consistency (REL-01 preflight).** Assert every version source
   agrees before tagging — workspace `Cargo.toml`, `pyproject`, the tst-py
   `Cargo.toml`, the C `TST_VERSION_MINOR` + committed `tstrans.h`, the JNI
   `versionString`, and the embedded `Cargo.lock`s:
   ```bash
   bash scripts/check/repo/release-version-consistency.sh
   ```
   It must report all-consistent (`0.2.0`) before you tag. The same rail runs
   in CI on the tag and gates both publish workflows.
2. **`main` is green** — `ci`, `jvm-jar`, and `python-wheels` all passing on
   the commit you intend to tag.
3. **Dry-run the wheels** (recommended for the first PyPI release):
   `gh workflow run python-wheels.yml` builds every wheel + sdist and skips
   publish (publish is tag-gated). Confirm all legs are green, and watch for a
   leg sitting *queued* — that is the v0.1.0 failure mode.

## Release procedure

1. **Tag and push** (only after explicit maintainer confirmation — Maven
   Central is immutable):
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```
2. The `v0.2.0` tag triggers both workflows:
   - **python-wheels.yml** builds wheels + sdist and **publishes to PyPI via
     OIDC trusted publishing.** This is the FIRST time the OIDC path runs —
     **watch the publish job.** If any required wheel leg is cancelled or
     stuck queued, the `needs:`-gated publish job is *skipped* (not failed);
     a skipped publish means nothing reached PyPI.
   - **jvm-jar.yml** builds the 4 native libs, assembles the fat jar, then the
     `publish` job uploads a signed bundle to the **Central Portal (staged)**.
3. **Release Maven manually:** https://central.sonatype.com → Deployments,
   inspect the staged `org.tstrans:tstrans-jvm:0.2.0` bundle (validation must
   pass), then click **Publish**. It appears on Maven Central shortly after.
4. **Verify BEFORE advertising:**
   - `pip install tstrans==0.2.0` resolves and imports.
   - A Gradle / Maven resolve of `org.tstrans:tstrans-jvm:0.2.0` succeeds.
   Only **after** both verify, flip the README / docs from "publishing to PyPI
   with v0.2.0" to "available."

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

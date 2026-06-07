# Releasing

The first public release bundles **PyPI** (`tstrans` wheels) and **Maven
Central** (`org.tstrans:tstrans-jvm`) on a single `v*` tag. PyPI publishes
automatically (OIDC trusted publishing); Maven uploads to the Central Portal
in a **staged** state that a maintainer releases manually.

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

   PyPI needs no secret — it uses OIDC trusted publishing, already configured.

## Release procedure

1. Ensure the version is right and `main` is green (`jvm-jar` + `ci` +
   `python-wheels` workflows).
2. Optional dry-run of the wheels: `gh workflow run python-wheels.yml`
   (builds all wheels, skips publish — publish is tag-gated).
3. Tag and push:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
4. The `v0.1.0` tag triggers both workflows:
   - **python-wheels.yml** builds wheels + sdist and **publishes to PyPI**
     automatically.
   - **jvm-jar.yml** builds the 4 native libs, assembles the fat jar, then the
     `publish` job uploads a signed bundle to the **Central Portal (staged)**.
5. **Release Maven manually:** go to https://central.sonatype.com → Deployments,
   inspect the staged `org.tstrans:tstrans-jvm:0.1.0` bundle (validation must
   pass), then click **Publish**. It appears on Maven Central shortly after.
6. Verify: `pip install tstrans==0.1.0` and a Gradle/Maven resolve of
   `org.tstrans:tstrans-jvm:0.1.0`.

## Notes

- Maven Central releases are **permanent/immutable** — a bad bundle means
  shipping a corrective `0.1.1`, never overwriting. That's why the first
  release stages for manual inspection (`automaticRelease = false` in
  `bindings/jvm/build.gradle.kts`); flip it to `true` for later releases once
  the pipeline is proven.
- If the Portal upload fails with an opaque error, re-run the publish with
  `--info` added to the `./gradlew publishToMavenCentral` invocation to capture
  the HTTP response body from the Portal.
- The fat jar covers linux-x86_64/aarch64, macos-aarch64, windows-x86_64.
  macOS Intel (`macos-x86_64`) is deferred — see
  [deferred-features.md](deferred-features.md).

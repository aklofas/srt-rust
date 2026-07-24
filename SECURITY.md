# Security Policy

ts-transformer parses untrusted wire input by design — MPEG-TS, KLV, and
elementary-stream bytes arriving over UDP, TCP, RTP/RTSP, SRT, RIST, or HLS.
Robustness issues in those parsers are security issues here: a reachable
panic, unbounded allocation, or hang on hostile input is a denial-of-service
against a receiver, even without a classical memory-safety impact.

## Reporting a vulnerability

**Please do not open a public issue for a suspected vulnerability.**

- **Preferred:** [GitHub private vulnerability reporting](https://github.com/aklofas/ts-transformer/security/advisories/new)
  — the *Report a vulnerability* button on the repository's Security tab.
- **Fallback:** email [aklofas@gmail.com](mailto:aklofas@gmail.com) with a
  subject line starting `[SECURITY]`.

Helpful details to include: the affected version (release tag or commit),
the component (crate, binding, or C ABI surface), a reproduction — hostile
input bytes, a packet capture, or a code snippet — and your assessment of
the impact.

## Supported versions

This project is pre-1.0. Security fixes land on `main` and ship in the next
release; they are not backported.

| Version                | Supported |
| ---------------------- | --------- |
| Latest release         | ✅        |
| Older releases         | ❌ — upgrade to the latest release |

## Response expectations

This is a single-maintainer project. You can expect an acknowledgment within
a few business days, and a triage verdict with a fix-or-timeline answer
shortly after. Coordinated disclosure is appreciated — please allow a fix
and a release before publishing details.

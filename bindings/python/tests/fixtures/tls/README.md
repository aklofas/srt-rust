# TLS test fixtures

Self-signed test certificate + key used by the TLS-path pytest suites
(`tcps://`, `rtsps://`, HTTPS HLS). NOT a secret — generated purely for
loopback tests, valid until 2126 so it never needs rotation:

```bash
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -keyout key.pem -out cert.pem -days 36500 -nodes -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
  -addext "basicConstraints=critical,CA:FALSE"
```

The cert is explicitly `CA:FALSE`: rustls (webpki) rejects a CA cert
presented as the server's end-entity certificate (`CaUsedAsEndEntity`),
while both rustls and Python `ssl` happily accept a self-signed non-CA
cert as a trust anchor.

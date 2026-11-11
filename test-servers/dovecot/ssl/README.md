# Test certificate for the Dovecot test server

`tls.key` is **public and test-only**: it is committed to the repository, anyone can read it, and
it must never be used for a real server. `tls.crt` is a leaf certificate (not a CA) for
`localhost` and `127.0.0.1`: `CA:FALSE`, key usage `digitalSignature,keyEncipherment`, extended
key usage `serverAuth`, valid 10 years. The integration tests pin its SHA-256 fingerprint (they
read it from `tls.crt`, so replacing the files needs no code change).

Regenerate:

```
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -keyout tls.key -out tls.crt -days 3650 -subj "/CN=localhost" \
  -addext "basicConstraints=critical,CA:FALSE" \
  -addext "keyUsage=critical,digitalSignature,keyEncipherment" \
  -addext "extendedKeyUsage=serverAuth" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
```

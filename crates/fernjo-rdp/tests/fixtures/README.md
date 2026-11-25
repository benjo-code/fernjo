# RDP TLS test certificates

Self-signed certificates and their private keys for the fake RDP server in `tests/support`
(`Behaviour::TlsWithCert`, `Behaviour::ImpersonatedWithCert`, `tests/no_root_store.rs`) and the unit tests of `src/tls.rs`. **Test-only keys**: they protect
nothing and are committed on purpose (`.gitignore` and `.gitleaks.toml` exempt exactly this
folder's `*.key` files). Code scanners may report them as private keys; they are test data.

| Files | What it is | Expected result |
| --- | --- | --- |
| `rsa2048-sha256.{crt,key}` | Like the certificate Windows creates for RDP: RSA 2048, SHA-256, key usage and server-auth EKU, v3 | reaches the certificate decision |
| `rsa2048-sha1.{crt,key}` | The same, signed with SHA-1 (the signature *on* the certificate) | reaches the certificate decision |
| `critical-ext.{crt,key}` | RSA 2048, SHA-256, plus an unknown extension marked critical | `certificate_unsupported` before any prompt |
| `ec-p521.{crt,key}` | ECDSA P-521 (the ring provider has no algorithm for that curve) | counted as unsupported (unit test) |
| `rsa1024-sha256.{crt,key}` | RSA 1024, SHA-256 (rustls's ring provider verifies RSA 2048 to 8192 bits only) | counted as unsupported (unit test; a rustls server cannot sign with this key, so there is no fake-server case) |

Generated with OpenSSL 3.5.9 (Git Bash, `/ucrt64/bin/openssl`) in this folder. `MSYS_NO_PATHCONV=1`
keeps Git Bash from turning `/CN=...` into a Windows path. OpenSSL 3.5 signed with SHA-1 at its
default security level; no configuration override was needed.

```bash
export MSYS_NO_PATHCONV=1
WIN='-addext keyUsage=keyEncipherment,dataEncipherment -addext extendedKeyUsage=serverAuth'
openssl req -x509 -newkey rsa:2048 -nodes -keyout rsa2048-sha256.key -out rsa2048-sha256.crt -sha256 -days 36500 -subj /CN=127.0.0.1 $WIN
openssl req -x509 -newkey rsa:2048 -nodes -keyout rsa2048-sha1.key -out rsa2048-sha1.crt -sha1 -days 36500 -subj /CN=127.0.0.1 $WIN
openssl req -x509 -newkey rsa:2048 -nodes -keyout critical-ext.key -out critical-ext.crt -sha256 -days 36500 -subj /CN=127.0.0.1 $WIN -addext "1.3.6.1.4.1.55555.1=critical,ASN1:NULL"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-521 -nodes -keyout ec-p521.key -out ec-p521.crt -sha256 -days 36500 -subj /CN=127.0.0.1 -addext keyUsage=digitalSignature -addext extendedKeyUsage=serverAuth
openssl req -x509 -newkey rsa:1024 -nodes -keyout rsa1024-sha256.key -out rsa1024-sha256.crt -sha256 -days 36500 -subj /CN=127.0.0.1 $WIN
```

Regenerating changes the keys and fingerprints; the tests compute fingerprints from the files, so
nothing else needs updating.

# Update signature fixtures: TEST ONLY

Inputs of the unit tests of `examples/verify_update_signature.rs`. Nothing here is used for a
release; the key pair of the real updates is not in this repository.

- `payload.bin` is the file that is signed (a copy of `src-tauri/tests/fixtures/update-test-key/payload.bin`).
- `test-only.pub` and `tauri.conf.json` hold the published TEST public key
  (`src-tauri/tests/fixtures/update-test-key/`, whose private half is published on purpose).
- `with-app-version.sig`: `payload.bin` signed with the test key and `--app-version 9.9.9`.
- `no-app-version.sig`: `payload.bin` signed with the test key without `--app-version`.
- `other-key.pub`, `other-conf.json`, `other-key.sig`: a second throw-away key pair generated for this
  folder (its private key was deleted) and `payload.bin` signed with it.

Regenerate with `pnpm tauri signer generate --ci -w <scratch>/other.key` and
`pnpm tauri signer sign [--app-version 9.9.9] <scratch>/payload.bin` (set
`TAURI_SIGNING_PRIVATE_KEY_PATH` and an empty `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`).

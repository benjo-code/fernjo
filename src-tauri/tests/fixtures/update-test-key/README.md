# Update test key: TEST ONLY

This key pair signs the fixture `payload.bin` for `src-tauri/tests/updater_feed.rs`.

- `test-only.privkey` is a **private key that is published on purpose**. Its password is empty. It is
  **never** used for a release and is not the key whose public half is pinned in
  `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`); the test `updater_config_is_pinned` checks that
  the two differ.
- `test-only.pub` is its public key (the text the plugin takes as `pubkey`).
- `payload.bin.sig` is the signature of `payload.bin` made with `--app-version 9.9.9`, so the signed
  version is bound to `9.9.9`.

Regenerate (only if the format changes):

    pnpm tauri signer generate --ci -w <abs path>/test-only.privkey
    head -c 8192 /dev/urandom > payload.bin
    TAURI_SIGNING_PRIVATE_KEY_PATH=<abs path>/test-only.privkey TAURI_SIGNING_PRIVATE_KEY_PASSWORD= \
      pnpm tauri signer sign --app-version 9.9.9 <abs path>/payload.bin

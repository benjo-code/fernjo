# Security policy

## Supported versions

fernjo is in early development. Security fixes are made for the latest release only (and for `dev` until the first release exists).

## Reporting a vulnerability

Please report vulnerabilities **privately**, not as a public issue or pull request:

1. Open the repository's **Security** tab and choose **Report a vulnerability**
   ([direct link](https://github.com/benjo-code/fernjo/security/advisories/new)). This uses GitHub's private vulnerability reporting.
2. Describe what you found, the affected version or commit, and how to reproduce it. Do not include real credentials, private keys or other people's data.

This is a personal open-source project, so responses are best effort: expect an acknowledgement within about a week. Fixes for confirmed issues are released as soon as practical, and reporters are credited in the advisory unless they prefer otherwise.

## What matters most

fernjo stores SSH and RDP credentials. Reports about the following are especially welcome:

- a secret (password, private key, passphrase, vault data key) reaching the webview, a log, `connections.json` or any file in plaintext
- weaknesses in the vault file format or its key handling
- bypasses of SSH host key verification
- anything that lets web content in the app window reach Tauri commands it should not

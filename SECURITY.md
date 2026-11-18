# Security policy

## Supported versions

fernjo is in early development. Security fixes are made for the latest release (from v0.1.0 on) and for `dev`; older releases are not patched.

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

## Threat model: the synced data folder

The data folder is meant to be synced (Syncthing, Nextcloud, iCloud). The vault file is encrypted and authenticated, but `connections.json` and `known_hosts` are plain files without integrity protection. Anyone who can write to the synced folder (another device, a sync server, a shared folder) can change a connection so that it points at a host of their choosing and add that host's key to `known_hosts`; the next connect then shows no host key prompt and sends the credential the connection references to that host. This is the trust model of OpenSSH's `~/.ssh/known_hosts` too, but a synced folder is easier to reach than a private home directory. The same writers can also put an older valid `vault.fernjo` back (there is no rollback protection). Use only sync folders that nobody but you can write to.

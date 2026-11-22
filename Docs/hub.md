# fernjo hub

The hub is the small server behind fernjo's multi-device sync. It stores one encrypted snapshot (the "head") and the last ten older ones. Design: `Docs/superpowers/specs/2026-11-22-m09-multi-device-sync-design.md`, decision 2.

## What the hub can and cannot see

- It sees: the size of each snapshot, when it was written, the `seq` counter and group id in the unencrypted 34-byte envelope header, the IP address of each writer, and (in Caddy's logs) request paths.
- It never sees: connection names, hosts, user names, vault contents, or the sync key. Snapshots are encrypted on the device with a key the hub never receives; the vault inside is additionally encrypted by the master password.
- It can: refuse requests, lose data, or withhold the newest head. It cannot forge a head that devices accept (the envelope is authenticated). The ten older snapshots are for your own rescue; there is no UI for them (they are files in `history/`).
- Only the SHA-256 of the hub secret is stored (`config.json`). A request must carry `Authorization: Bearer <secret>`.

## Build the image

From the repository root:

```sh
podman build -f crates/fernjo-hub/Containerfile -t fernjo-hub .
```

The result is a static musl binary on an empty base image, running as the unprivileged user 10001. It listens on port 8787 inside the container and writes only to `/data`.

## Run it rootless under Podman (Quadlet)

1. Create the data volume and the secret once (the secret is printed once, copy it into your password manager):

   ```sh
   podman volume create fernjo-hub-data
   podman run --rm --user 10001:10001 --userns=keep-id:uid=10001,gid=10001 \
     -v fernjo-hub-data:/data:Z localhost/fernjo-hub:latest init --data /data
   ```

2. Copy `crates/fernjo-hub/deploy/fernjo-hub.container` to `~/.config/containers/systemd/`, then:

   ```sh
   systemctl --user daemon-reload
   systemctl --user start fernjo-hub
   loginctl enable-linger "$USER"   # keep it running without a login
   ```

   The unit publishes `127.0.0.1:8787` only, mounts the root file system read-only, drops all capabilities and keeps the data in the named volume.

3. Check: `curl -H "Authorization: Bearer <secret>" http://127.0.0.1:8787/v1/info` answers `{"protocol":1,"maxBytes":67108864}`.

Without Podman the same binary runs anywhere: `fernjo-hub serve --data <dir> --listen 127.0.0.1:8787` (after `fernjo-hub init --data <dir>`). `serve` refuses to start in a folder that was not initialised.

## Caddy

Add the snippet from `crates/fernjo-hub/deploy/Caddyfile.example`:

```
benjo.loc {
	tls internal
	reverse_proxy 127.0.0.1:8787
}
```

fernjo only talks HTTPS (plain `http://` is accepted for loopback only), so each device must trust Caddy's internal root certificate. Get it on the Caddy host with either of:

- `caddy trust` (installs it on that host only), or
- copy `/data/caddy/pki/authorities/local/root.crt` out of the Caddy container (the path inside the official image; on a bare install it is under `~/.local/share/caddy/pki/authorities/local/root.crt`).

Paste the PEM text of `root.crt` into the "CA certificate" field when you set up sync in fernjo (it travels in the pairing code to your other devices). Do not install it system-wide on devices if you want trust limited to fernjo.

## Secrets

- `fernjo-hub init --data <dir>` creates `config.json` and prints the secret once. It refuses to run in a folder that already has one.
- `fernjo-hub rotate-secret --data <dir>` prints a new secret and replaces the stored hash. A running `serve` reads the hash at start, so restart it afterwards (`systemctl --user restart fernjo-hub`). Every device then needs the new secret ("Edit hub secret" in fernjo). This is also how you lock out a lost device.

## Rebuild an empty hub (Re-seed)

If the hub's data is lost, restored from an older backup, or the volume is recreated, devices refuse it (`hub_rolled_back`). Fix the hub first (`init` for a new volume), then in fernjo on the device that has the data you want, choose "Re-seed hub from this device". The other devices catch up on their next sync.

## History files

Each replaced head is kept as `history/<seq>.env`, the newest ten. They are encrypted envelopes, so reading one needs the sync key; to roll back, copy one over `head.env` while the hub is stopped, and use "Re-seed" afterwards if devices complain. Writes are atomic (random temp name, fsync, rename), and a stale write (HTTP 412) leaves head and history untouched.

## API (protocol 1)

| Request | Meaning |
| --- | --- |
| `GET /v1/head` | 200 with the envelope and `ETag: "<seq>"`, or 404 when empty |
| `PUT /v1/head` with `If-None-Match: *` | first write; 201 |
| `PUT /v1/head` with `If-Match: "<seq>"` | replace the head; 204 |
| `GET /v1/info` | `{"protocol":1,"maxBytes":67108864}` |

`PUT` errors: 401 (bad or missing secret), 400 (not an envelope, bad precondition header), 409 (`seq` not higher, or another group), 412 (the head is not the one expected), 413 (over 64 MiB, refused without reading the rest), 428 (no precondition header).

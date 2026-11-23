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

`docker build -f crates/fernjo-hub/Containerfile -t fernjo-hub .` works the same. The `.dockerignore` at the repository root keeps `target/`, `node_modules/`, `.git/` and similar out of the build context (without it the context is tens of gigabytes); Podman reads that file too.

The result is a static musl binary on an empty base image, running as the unprivileged user 10001. It listens on port 8787 inside the container and writes only to `/data`. The image contains an empty `/data` owned by 10001, so a new named volume mounted there is writable by that user without a `chown`. The build stage uses `rust:1.100.0-alpine`, the version `rust-toolchain.toml` pins; `rustup` still downloads the `rustfmt` and `clippy` components listed there during the build.

Stopping the container (`podman stop`, `docker stop`, `systemctl --user stop fernjo-hub`) sends SIGTERM; `fernjo-hub serve` handles it (and SIGINT), finishes a write that is already running and exits with status 0 within a second.

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

   The unit publishes `127.0.0.1:8787` only, mounts the root file system read-only, drops all capabilities, runs a tiny init as PID 1 (`RunInit=true`, belt and braces next to the hub's own signal handling) and keeps the data in the named volume.

   The supported rootless path is the one in these two steps: `--userns=keep-id:uid=10001,gid=10001` (and `UserNS=` in the unit) maps your user onto 10001, so the volume that the image's `/data` seeded stays writable for the hub. Whether that mapping works on your Podman version is an owner check (see the end of this page).

3. Check: `curl -H "Authorization: Bearer <secret>" http://127.0.0.1:8787/v1/info` answers `{"protocol":1,"maxBytes":67108864}`.

Without Podman the same binary runs anywhere: `fernjo-hub serve --data <dir> --listen 127.0.0.1:8787` (after `fernjo-hub init --data <dir>`). `serve` refuses to start in a folder that was not initialised.

Run exactly one `serve` process per data directory: the compare-and-swap lock is inside the process, so two hubs on one folder could both accept a write against the same head.

## Command line

```
fernjo-hub init --data <dir>
fernjo-hub serve --data <dir> --listen <addr:port>
fernjo-hub rotate-secret --data <dir>
```

- `--data` is required by every command; `--listen` must be an IP address with a port (`127.0.0.1:8787`, `0.0.0.0:8787`, `[::1]:8787`), a host name is not accepted. A missing command or flag, an unknown command or an unparsable address prints the usage text to stderr and exits with 1.
- `init` creates the folder and `config.json` and prints `Hub secret (shown once, store it safely): <secret>` on stdout. The secret is 32 random bytes in base64url (43 characters). Exit 0; exit 1 with a message on stderr when the folder already holds a `config.json` ("already initialised; use rotate-secret").
- `rotate-secret` prints `New hub secret (shown once): <secret>` and a reminder to restart `serve`. Exit 1 when the folder was not initialised or `config.json` is damaged (also when larger than 4 KiB).
- `serve` prints `fernjo-hub listening on <addr>` to stderr and runs until it receives SIGINT (Ctrl+C) or, on Unix, SIGTERM, prints `fernjo-hub stopping`, lets a write that is already running finish and exits with 0 (every write is atomic as well, so a kill at any moment is also safe). Exit 1 with a message when the folder was not initialised, `config.json` is damaged, the data folder cannot be prepared or the port cannot be bound. At start it creates `history/` (mode 0700 on Unix) and removes `*.tmp` leftovers of a crashed write. Nothing is logged per request: no path, header, body or secret.
- Files in `<dir>`: `config.json` (`{"secretSha256": "<hex>"}`), `head.env`, `history/<seq>.env`.

## Caddy

Add the snippet from `crates/fernjo-hub/deploy/Caddyfile.example`:

```
benjo.loc {
	tls internal
	reverse_proxy 127.0.0.1:8787
}
```

If Caddy itself runs in a container, `127.0.0.1` inside that container is the container, not the host: point `reverse_proxy` at the host's address instead (the host's LAN address, `host.containers.internal` under Podman (untried), or put both containers on one Podman network and use the hub container's name and port 8787). Because the unit publishes `127.0.0.1:8787` only, a Caddy container that must reach the hub over the host's LAN address needs `PublishPort=<host address>:8787:8787` in the unit instead; keep it off any address that is reachable from outside the home network, because the hub itself speaks plain HTTP.

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

Wire formats, the envelope layout and the merge rules are in `Docs/sync-format.md`; that page is enough to write a compatible hub or client.

## Verified so far

- The hub and the sync client were tested with in-process tests (the hub on loopback, two clients against it, fake servers for the client's refusals).
- **Built and run with Docker (not Podman)**: the image builds from a clean `git archive` of the repository (context about 0.3 MB with the root `.dockerignore`); `fernjo-hub init --data /data` succeeds as user 10001 on a fresh Docker named volume; `serve` runs, and `docker stop` ends it in about half a second with exit status 0 (before the signal handler it took 7 s and ended with 137). The hub's Unix test of SIGTERM and SIGINT (`crates/fernjo-hub/tests/shutdown.rs`) ran in a Linux container.

## Not verified yet (owner checks)

Podman rootless and the Quadlet unit were **not run**; Docker is not a check of either. Still open, to be run by the owner:

- `podman build -f crates/fernjo-hub/Containerfile -t fernjo-hub .` and the rootless Quadlet unit on the Ubuntu or Debian VM (the `keep-id` user mapping, `RunInit=true`, the `:Z` label on a named volume and the read-only root file system have not been run under Podman).
- Caddy with `tls internal` in front of the hub, including a Caddy restart and a hub restart that lose nothing, with the CA pasted into fernjo.
- Two real devices syncing through that hub (create, edit, delete, conflict copy, vault entry, offline then online), and the OS keychain items on Windows, macOS and Linux.
- The `host.containers.internal` address and a LAN `PublishPort` for a Caddy that runs in a container (named under "Caddy" as untried advice).

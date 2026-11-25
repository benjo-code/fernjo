# fernjo hub

The hub is the small server behind fernjo's multi-device sync. It stores one encrypted snapshot (the "head") and the last ten older ones. Design: `Docs/superpowers/specs/2026-11-22-m09-multi-device-sync-design.md`, decision 2.

## What the hub can and cannot see

- It sees: the size of each snapshot, when it was written, the `seq` counter and group id in the unencrypted 34-byte envelope header, the IP address of each writer, and (in Caddy's logs) request paths.
- It never sees: connection names, hosts, user names, vault contents, or the sync key. Snapshots are encrypted on the device with a key the hub never receives; the vault inside is additionally encrypted by the master password.
- It can: refuse requests, lose data, or withhold the newest head. It cannot forge a head that devices accept (the envelope is authenticated). The ten older snapshots are for your own rescue; there is no UI for them (they are files in `history/`).
- Only the SHA-256 of the hub secret is stored (`config.json`). A request must carry `Authorization: Bearer <secret>`.

## Set it up from the published image

Every release from v0.3.1 on publishes the hub as a ready-made image, `ghcr.io/benjo-code/fernjo-hub`, for `linux/amd64` and `linux/arm64` (a Raspberry Pi 4 or 5, an ARM VM, or an ordinary x86 server). Nothing has to be built. The tags are `X.Y.Z` (one release; pin this one), `X.Y` (the latest patch of a minor version) and `latest`. There is no image for v0.3.0 and earlier; build it yourself (see "Build the image yourself" below).

The image is a static musl binary on an empty base image, running as the unprivileged user 10001. It listens on port 8787 inside the container and writes only to `/data`. It contains an empty `/data` owned by 10001, so a new named volume mounted there is writable by that user without a `chown`.

Pick one of the three ways below. All of them first create the hub's secret once; it is printed once, so copy it into your password manager.

### Compose (recommended)

The files are in `crates/fernjo-hub/deploy/`: `compose.yaml` and, for rootless Podman, `compose.podman.yaml`. They publish `127.0.0.1:8787` only, keep the data in the named volume `hub-data`, run the image as 10001 with a read-only root file system, no capabilities and a tiny init, and restart it unless you stop it.

With Docker:

```sh
cd crates/fernjo-hub/deploy
docker compose -f compose.yaml run --rm hub init --data /data   # once; prints the secret
docker compose -f compose.yaml up -d
```

With rootless Podman, add the override (it maps your user onto 10001 so the volume stays writable; Docker refuses that key, which is why it is a separate file):

```sh
cd crates/fernjo-hub/deploy
podman compose -f compose.yaml -f compose.podman.yaml run --rm hub init --data /data
podman compose -f compose.yaml -f compose.podman.yaml up -d
```

`compose run` does not publish the port, so the one-time `init` never collides with a running hub. Check with `curl -H "Authorization: Bearer <secret>" http://127.0.0.1:8787/v1/info`, which answers `{"protocol":1,"maxBytes":67108864}`. `compose down` stops it and keeps the volume.

### Rootless Podman as a service (Quadlet)

1. Create the data volume and the secret once:

   ```sh
   podman volume create fernjo-hub-data
   podman run --rm --user 10001:10001 --userns=keep-id:uid=10001,gid=10001 \
     -v fernjo-hub-data:/data:Z ghcr.io/benjo-code/fernjo-hub:0.3.1 init --data /data
   ```

2. Copy `crates/fernjo-hub/deploy/fernjo-hub.container` to `~/.config/containers/systemd/`, then:

   ```sh
   systemctl --user daemon-reload
   systemctl --user start fernjo-hub
   loginctl enable-linger "$USER"   # keep it running without a login
   ```

   The image is pulled on the first start. The unit publishes `127.0.0.1:8787` only, mounts the root file system read-only, drops all capabilities, runs a tiny init as PID 1 (`RunInit=true`, belt and braces next to the hub's own signal handling) and keeps the data in the named volume.

   `--userns=keep-id:uid=10001,gid=10001` (and `UserNS=` in the unit) maps your user onto 10001, so the volume that the image's `/data` seeded stays writable for the hub. The owner ran the unit with a locally built image on Podman 5.7.0 (see "Verified so far").

3. Check with the same `curl` as above.

### One container, no files

```sh
docker volume create fernjo-hub-data
docker run --rm --user 10001:10001 -v fernjo-hub-data:/data ghcr.io/benjo-code/fernjo-hub:0.3.1 init --data /data
docker run -d --name fernjo-hub --restart unless-stopped --user 10001:10001 --read-only --cap-drop ALL \
  --security-opt no-new-privileges --init -p 127.0.0.1:8787:8787 -v fernjo-hub-data:/data \
  ghcr.io/benjo-code/fernjo-hub:0.3.1
```

(`podman run` takes the same arguments; add `--userns=keep-id:uid=10001,gid=10001` when rootless.)

Without a container the same binary runs anywhere: `fernjo-hub serve --data <dir> --listen 127.0.0.1:8787` (after `fernjo-hub init --data <dir>`). `serve` refuses to start in a folder that was not initialised.

Run exactly one `serve` process per data directory: the compare-and-swap lock is inside the process, so two hubs on one folder could both accept a write against the same head.

Stopping the container (`podman stop`, `docker stop`, `compose down`, `systemctl --user stop fernjo-hub`) sends SIGTERM; `fernjo-hub serve` handles it (and SIGINT), finishes a write that is already running and exits with status 0 within a second.

## Update to a new hub version

The deploy files pin the exact version (`0.3.1` above), so nothing changes until you decide. To update, change the tag in the file you use (`compose.yaml`, `fernjo-hub.container` or your `run` command) to the new release, pull and restart; the data volume and the secret stay as they are:

```sh
docker compose -f compose.yaml pull && docker compose -f compose.yaml up -d      # Compose
podman pull ghcr.io/benjo-code/fernjo-hub:<new version> && systemctl --user restart fernjo-hub   # Quadlet
```

Every release also bumps the tag in both deploy files to its own version, so the files in a newer checkout already name the newer image. The hub protocol is versioned (`/v1/info`); the app tells you when a hub is older than the device (`Sync: hub older than this device`).

## Verify the image

The release workflow attests the build provenance of the image. Check that the image you pulled was built by this repository's release workflow:

```sh
gh attestation verify oci://ghcr.io/benjo-code/fernjo-hub:0.3.1 --repo benjo-code/fernjo \n  --signer-workflow benjo-code/fernjo/.github/workflows/release.yml --source-ref refs/tags/v0.3.1
```

The two extra flags matter: without them any workflow of this repository on any branch would pass. Use the tag of the version you pulled (replace `0.3.1` in this command and in the examples above when you use another version).

## Make the package public (once, owner)

GitHub creates the package `fernjo-hub` private on the first push. So that everybody can pull it without logging in, the repository owner changes its visibility once: on GitHub open the package (github.com/benjo-code, Packages, `fernjo-hub`), Package settings, Change visibility, Public. Check it from a machine that is not logged in to GHCR: `docker pull ghcr.io/benjo-code/fernjo-hub:0.3.1`. Also check once that the package is connected to the repository (the package page shows `benjo-code/fernjo`, and Package settings, Manage Actions access, lists it with Write): the workflow annotates the image index with its source for that, but older Podman versions cannot (the run then prints a warning), and an unconnected package can refuse the next release's push. Connect it there if it is missing.

## Build the image yourself

From the repository root (also the way to run a version without a published image):

```sh
podman build -f crates/fernjo-hub/Containerfile -t fernjo-hub .
```

`docker build -f crates/fernjo-hub/Containerfile -t fernjo-hub .` works the same. The `.dockerignore` at the repository root keeps `target/`, `node_modules/`, `.git/` and similar out of the build context (without it the context is tens of gigabytes); Podman reads that file too. `--build-arg VERSION=<x.y.z> --build-arg REVISION=<commit>` fill the image labels (defaults `dev` and `unknown`). The build stage uses `rust:1.100.0-alpine`, the version `rust-toolchain.toml` pins; `rustup` still downloads the `rustfmt` and `clippy` components listed there during the build. Use `localhost/fernjo-hub:latest` instead of the GHCR reference in the commands above.

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

fernjo only talks HTTPS (a release build refuses plain `http://` also to `127.0.0.1`; only debug builds accept it there, for the tests), so each device must trust Caddy's internal root certificate. Get it on the Caddy host with either of:

- `caddy trust` (installs it on that host only), or
- copy `/data/caddy/pki/authorities/local/root.crt` out of the Caddy container (the path inside the official image; on a bare install it is under `~/.local/share/caddy/pki/authorities/local/root.crt`).

Paste the PEM text of `root.crt` into the "CA certificate" field when you set up sync in fernjo (it travels in the pairing code to your other devices). Do not install it system-wide on devices if you want trust limited to fernjo.

## Secrets

- `fernjo-hub init --data <dir>` creates `config.json` and prints the secret once. It refuses to run in a folder that already has one.
- `fernjo-hub rotate-secret --data <dir>` prints a new secret and replaces the stored hash. A running `serve` reads the hash at start, so restart it afterwards (`systemctl --user restart fernjo-hub`). Every device then needs the new secret ("Edit hub secret" in fernjo). This is also how you lock out a lost device.

## Rebuild an empty hub (Re-seed)

If the hub's data is lost, restored from an older backup, or the volume is recreated, devices refuse it (`hub_rolled_back`). Fix the hub first (`init` for a new volume), then in fernjo on the device that has the data you want, choose "Re-seed hub from this device". The other devices catch up on their next sync.

## History files

Each replaced head is kept as `history/<seq>.env`, the newest ten. They are encrypted envelopes, so reading one needs the sync key; to roll back, copy one over `head.env` while the hub is stopped (a copy, not a symbolic link: the hub refuses a `head.env` that is a link or larger than 64 MiB), and use "Re-seed" afterwards if devices complain. Writes are atomic (random temp name, fsync, rename), and a stale write (HTTP 412) leaves head and history untouched.

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
- **Run by the owner on 2026-11-23** (reported by the owner, without per-step detail): the image built with `podman build` and the rootless Quadlet unit ran on a VM (Podman 5.7.0, `UserNS=keep-id:uid=10001,gid=10001`, `RunInit=true`, the `:Z` volume, the read-only root file system). Caddy with `tls internal` ran on a separate VM in front of it, with the hub published on the hub VM's LAN address (`PublishPort=<LAN address>:8787:8787`) and the CA pasted into fernjo. A Windows 11 PC and a macOS laptop synced through it: create, edit, delete versus edit, conflict copy, vault entry, offline then online, a restart of the hub container and Caddy, a hub restored from an older snapshot and re-seeded, and `rotate-secret`. The OS keychain items worked on Windows and macOS. Two defects came out of it and were fixed: the unit used the key `Init=`, which Quadlet rejects (the key is `RunInit=`), and the notice for a deleted-versus-edited item said "changed here" also on the device that had deleted it.
- **M12 (published image), pull request dry run of 2026-11-25** (pull request 57, run 45709519618): the release workflow built the image on a native amd64 runner and a native arm64 runner with rootless Podman, and the smoke test printed `smoke: ok` on both (`init` on a fresh volume as 10001, `serve`, `/v1/info` with the secret, 401 without it, stop in under 5 s with exit status 0); the push job was skipped, so nothing was published.
- **M12, Docker Compose on Docker Desktop (Docker 29.9.3)**, with the locally built image tagged as the pinned reference: `docker compose -f compose.yaml config` is valid (also merged with `compose.podman.yaml`), `compose run --rm hub init --data /data` printed the secret, `up -d` started the hub with the read-only root file system, and `/v1/info` without a secret answered 401.
- **M12, the push step rehearsed** against a local registry, with the two OCI archives from the dry run and Podman 4.9.3 (the version on the runners) and 6.2.0: the same commands as the workflow joined them into one manifest list, pushed it as `X.Y.Z`, `X.Y` and `latest` with one shared digest, and `skopeo inspect --raw` showed amd64 and arm64. This is not a run against GHCR.

## Not verified yet (owner checks)

- The published image itself: it exists only after the v0.3.1 tag. Then: `docker buildx imagetools inspect ghcr.io/benjo-code/fernjo-hub:0.3.1` lists both architectures, `X.Y` and `latest` show the same digest, and the `gh attestation verify oci://... --signer-workflow ... --source-ref refs/tags/v0.3.1` command above passes. The login to GHCR and the attestation with `push-to-registry` have only run on a real tag push, never before.
- That the package can be pulled without a login once the owner has made it public ("Make the package public" above).
- The Compose files and the Quadlet unit with the published image on the owner's Podman machines, and `podman compose -f compose.yaml -f compose.podman.yaml` (Podman's Compose was not run here, only Docker's).
- Linux: the Secret Service keychain item (set up, restart, leave sync, item gone) and a sync run from a Linux device (the CachyOS laptop).
- That `systemctl --user stop fernjo-hub` ends the Podman unit within about a second (not reported).
- Caddy running in a container: the `host.containers.internal` address and a shared Podman network, and a LAN `PublishPort` for a Caddy container (named under "Caddy" as untried advice). The tested set-up has Caddy on another VM.

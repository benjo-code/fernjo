# Milestone 12: Hub image on GitHub Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `v*` tag push publishes the sync hub as a multi-arch image `ghcr.io/benjo-code/fernjo-hub` (tags `X.Y.Z`, `X.Y`, `latest`, with provenance), and a Compose file and an updated Quadlet unit run that image.

**Architecture:** `release.yml` gets two jobs. `hub-image-build` (matrix: amd64 on `ubuntu-latest`, arm64 on `ubuntu-24.04-arm`, `contents: read` only) builds `crates/fernjo-hub/Containerfile` with Podman, smoke-tests the built image through `tools/smoke-hub-image.sh`, and uploads the image as an OCI archive; it also runs on pull requests as the dry run. `hub-image` (the only job with `packages: write`, `id-token`, `attestations`; tag push only) downloads both archives, joins them into one manifest list, pushes it to the three tags and attests the digest, so the bits that were tested are the bits that are pushed. Everything else (tag logic, version pins, deploy files) lives in small bash scripts and files covered by `tools/test-release-scripts.sh`.

**Tech Stack:** GitHub Actions (only actions already pinned in `release.yml`), Podman/skopeo preinstalled on GitHub-hosted Ubuntu runners, bash, Compose spec, Quadlet, `gh attestation`.

**Spec:** There is no separate spec: the milestone's task list and acceptance criteria in `Docs/roadmap.json` (rendered in `Docs/ROADMAP.md`, "Milestone 12 – Hub image on GitHub") are the requirements. Background: `Docs/hub.md`, `crates/fernjo-hub/Containerfile`, `crates/fernjo-hub/deploy/`, `Docs/DECISIONS.md` ("M6 release workflow ...", "M11: immutable releases, draft then publish"), `CLAUDE.md` ("Release workflow", "Release scripts", "Releases").

## Global Constraints

- Registry and name: `ghcr.io/benjo-code/fernjo-hub`; tags `X.Y.Z`, `X.Y`, `latest`, pushed only on a `v*` tag; architectures `linux/amd64` and `linux/arm64`, each built on a native GitHub-hosted runner, joined into one manifest list.
- Only the `hub-image` job gets `packages: write` (plus `id-token: write` and `attestations: write`); the workflow default stays `permissions: contents: read`; actions stay pinned to 40-hex SHAs and **no new action is introduced** (reuse the `checkout`, `upload-artifact`, `download-artifact`, `attest-build-provenance` pins already in `release.yml`); `bash tools/check-workflows.sh` stays green, so no `${{ }}` of `github.ref_name`, `github.ref`, `steps.*` or `needs.*` inside a `run:` (use `env:`), and workflows stay plain block style.
- Untrusted values (tag) reach scripts only through `env:`; scripts take the tag as one argument and match it with a regex (`^v?[0-9]+\.[0-9]+\.[0-9]+$`), never evaluate it.
- The dry run pushes nothing, needs no token write access and works for a fork pull request (no artifact upload from a fork, same condition as the installers).
- Smoke test: from the built image run `init` on a fresh volume as user `10001:10001`, start `serve`, answer `GET /v1/info` with the hub secret, stop the container with exit status 0 in about a second; a failing smoke test blocks the push.
- Image stays a static musl binary on `scratch`, user `10001`, empty `/data`; OCI labels `org.opencontainers.image.source`, `version`, `revision`, `licenses` = `MIT OR Apache-2.0`, `description`.
- Compose: `ghcr.io/benjo-code/fernjo-hub:<pinned X.Y.Z>`, `127.0.0.1:8787` published, named volume on `/data`, `user: 10001:10001`, read-only root file system, `cap_drop: [ALL]`, `no-new-privileges`, `init: true`, `restart: unless-stopped`; the rootless-Podman mapping `keep-id:uid=10001,gid=10001` only in `compose.podman.yaml`. Quadlet: same pinned tag, still rootless-Podman capable.
- The pinned tag in `compose.yaml` and `fernjo-hub.container` is `0.3.1` (no image exists for `v0.3.0`; its tag and workflow are fixed). Every later release bumps both tags together with `Cargo.toml` and `package.json`.
- Language: docs, comments, commit messages in English; Conventional Commits; stage files selectively; no `git add .`; no attribution lines in commit messages.
- Do not claim what was not run: anything that needs the real GHCR, a real tag, rootless Podman on the owner's machines or Docker Compose stays an open check with its command.

## Review Focus

Failure modes the roadmap implies but no single task's happy path exercises; each is pinned by a test in the task named in brackets.

1. A tag that is not plain `MAJOR.MINOR.PATCH` (`v0.3.1-rc.1`, `v1.2`, `v1.2.3; touch x`) must produce no image tags and a non-zero exit, so a stray tag never moves `latest` [Task 1].
2. The two deploy files pin different tags, or a tag that differs from the release version: the release build must fail before anything is pushed [Task 5].
3. A smoke test that hangs (hub does not answer, container does not stop) must time out and still remove its container and volume, so a stuck run cannot leak resources or hold the push forever [Task 3].
4. A fork pull request: no step may need the registry, the write token or artifact upload; and a pull request must never reach the push job [Task 6].
5. A re-run of the failed `hub-image` job after a partial push: pushing the same manifest list again must give the same digest on all three tags, and the job must fail loudly when the digests differ [Task 6].
6. `compose.yaml` used alone under Docker must contain no Podman-only key (Docker rejects `userns_mode: keep-id:...`), and `init` must not publish the port or keep the container [Task 4].

---

### Task 1: Tag computation script

**Files:**
- Create: `tools/hub-image-tags.sh`
- Modify: `tools/test-release-scripts.sh` (new section `# --- hub-image-tags.sh ---`, header comment updated to name the script)

**Interfaces:**
- Produces: `bash tools/hub-image-tags.sh <tag> <image>` prints one fully qualified reference per line, in this order: `<image>:X.Y.Z`, `<image>:X.Y`, `<image>:latest`; exit 0. Exit 1 with `error: tag is not a valid version (expected vMAJOR.MINOR.PATCH)` on stderr for anything else; exit 2 on a wrong argument count. `<image>` is passed in, never hard-coded, so the test needs no registry.

- [ ] **Step 1: Write the failing tests** in `tools/test-release-scripts.sh` using `assert_exit`:
  - `hub_image_tags_for_a_plain_version`: `v0.3.1` and `ghcr.io/x/y` exit 0, output equals the three lines `ghcr.io/x/y:0.3.1`, `ghcr.io/x/y:0.3`, `ghcr.io/x/y:latest` (compare the whole output through a small helper, since `assert_exit` only checks a substring; assert each line is present and `wc -l` is 3).
  - `hub_image_tags_accept_the_tag_without_v`: `0.3.1` gives the same.
  - Rejections with exit 1 and needle `not a valid`: `v0.3.1-rc.1`, `v1.2`, `v1.2.3.4`, `v01.2.3` is **accepted by the existing regex pattern style**, so do not test it; test `'v1.2.3; touch '"$marker"` and `'$(touch '"$marker"')'` and assert the marker file does not exist afterwards (copy the pattern of `check_release_version_does_not_evaluate_the_tag`).
  - `hub_image_tags_wrong_argument_count`: exit 2.

- [ ] **Step 2: Run** `bash tools/test-release-scripts.sh` — the new cases FAIL (script missing).

- [ ] **Step 3: Implement `tools/hub-image-tags.sh`** in the style of `tools/check-release-version.sh` (`set -euo pipefail`, same regex, usage header comment). Split the version with bash parameter expansion, no `eval`.

- [ ] **Step 4: Run** `bash tools/test-release-scripts.sh` — all PASS.

- [ ] **Step 5: Commit**

```bash
git add tools/hub-image-tags.sh tools/test-release-scripts.sh
git commit -m "ci: compute the hub image tags from a release tag"
```

---

### Task 2: Image metadata in the Containerfile

**Files:**
- Modify: `crates/fernjo-hub/Containerfile`
- Modify: `tools/test-release-scripts.sh` (section `# --- hub Containerfile ---`)

**Interfaces:**
- Consumes: nothing.
- Produces: the Containerfile accepts `--build-arg VERSION=<text>` and `--build-arg REVISION=<text>` (both default to `dev` / `unknown`, so the documented plain `podman build -f crates/fernjo-hub/Containerfile -t fernjo-hub .` still works) and the final image carries the five OCI labels. Task 3 and Task 6 pass these args.

- [ ] **Step 1: Write the failing static tests** (the real build is Step 4): `containerfile_final_stage_is_scratch_as_10001` asserts the file still contains `FROM scratch`, `USER 10001:10001`, `VOLUME /data` and the `COPY --from=build --chown=10001:10001 /data /data` line; `containerfile_has_the_oci_labels` asserts the file mentions each of the five label keys, `licenses="MIT OR Apache-2.0"` and `source="https://github.com/benjo-code/fernjo"`. Implement them as `grep -Fq` helpers used with `assert_true`.

- [ ] **Step 2: Run** `bash tools/test-release-scripts.sh` — the label test FAILS, the scratch/10001 test passes (it pins today's behaviour).

- [ ] **Step 3: Edit the Containerfile**: after the final `FROM scratch`, declare `ARG VERSION=dev` and `ARG REVISION=unknown` (ARGs must be declared inside the stage that uses them) and one `LABEL` instruction with the five keys; `description` is "Sync hub for fernjo: stores one encrypted snapshot and ten older ones". Update the header comment with the new build args and the pull path pointer to `Docs/hub.md`.

- [ ] **Step 4: Build and inspect** (needs Docker or Podman; if neither runs on this machine, leave the row open in the evidence): `docker build -f crates/fernjo-hub/Containerfile --build-arg VERSION=0.0.0-test --build-arg REVISION=abc123 -t fernjo-hub:test .` then `docker image inspect fernjo-hub:test --format '{{json .Config.Labels}}'` shows the five labels with those values, and `... --format '{{.Config.User}}'` prints `10001:10001`.

- [ ] **Step 5: Run** `bash tools/test-release-scripts.sh` — PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/fernjo-hub/Containerfile tools/test-release-scripts.sh
git commit -m "feat(hub): label the hub image with its source, version and licence"
```

---

### Task 3: Smoke test script

**Files:**
- Create: `tools/smoke-hub-image.sh`
- Modify: `tools/test-release-scripts.sh` (section `# --- smoke-hub-image.sh ---`)

**Interfaces:**
- Consumes: an image reference that Task 2's Containerfile produced.
- Produces: `ENGINE=<podman|docker> bash tools/smoke-hub-image.sh <image>` (default engine `docker`; refuses any other value with exit 2). Exit 0 and `smoke: ok` when: `init --data /data` as `--user 10001:10001` on a fresh uniquely named volume prints the secret line (`Hub secret (shown once, store it safely): <secret>`, the secret is 43 base64url characters); `serve` started detached with `127.0.0.1` published on a free host port prints `fernjo-hub listening on` within 10 s; `curl -fsS -H "Authorization: Bearer <secret>" http://127.0.0.1:<port>/v1/info` answers a body equal to `{"protocol":1,"maxBytes":67108864}`; the same request without a secret answers 401; `<engine> stop` returns and `<engine> inspect --format '{{.State.ExitCode}}'` is `0` and the stop took under 5 s (the roadmap says about a second; 5 s is the failure bar so a slow runner does not flake while the old 137-after-7-s behaviour still fails). Any failure exits 1 naming the step, prints the container's logs, and a `trap` always removes the container and the volume. The script never prints the secret except as part of the failed-step log it already shows from `init` (do not echo it on success; this script runs in a public CI log, and the secret belongs to a throwaway volume).

- [ ] **Step 1: Write the failing tests**: `smoke_hub_image_needs_an_image` (no argument: exit 2, `usage`), `smoke_hub_image_refuses_an_unknown_engine` (`ENGINE=rm`: exit 2), and `smoke_hub_image_fails_and_cleans_up_with_a_fake_engine`: put a stub `docker` first on `PATH` (a script in `$tmp/bin` that logs its arguments to a file and exits 1 for `run`), run the smoke script, assert exit 1, the output names the failing step, and the log shows `rm` for the volume and container that were created (that pins the cleanup trap without a real engine).

- [ ] **Step 2: Run** `bash tools/test-release-scripts.sh` — the three new cases FAIL.

- [ ] **Step 3: Implement `tools/smoke-hub-image.sh`**: `set -euo pipefail`, `trap cleanup EXIT`, unique names from `$$` plus `date +%s`, host port chosen by letting the engine pick (`-p 127.0.0.1::8787`, read back with `<engine> port`), each polling loop bounded (10 s) with `sleep 0.5`, every external call that can hang wrapped in `timeout`. Keep the secret in a shell variable passed to `curl` via a `-H @file`-free header; do not use `set -x`.

- [ ] **Step 4: Run the tests** — PASS. Then run it for real against the image from Task 2 Step 4: `bash tools/smoke-hub-image.sh fernjo-hub:test` prints `smoke: ok` and `docker ps -a` / `docker volume ls` show no leftovers (if no engine runs locally, record that the real run is left to the CI dry run, Task 6 Step 7).

- [ ] **Step 5: Commit**

```bash
git add tools/smoke-hub-image.sh tools/test-release-scripts.sh
git commit -m "test: smoke-test the hub image (init, serve, info, stop)"
```

---

### Task 4: Compose files and the Quadlet unit

**Files:**
- Create: `crates/fernjo-hub/deploy/compose.yaml`, `crates/fernjo-hub/deploy/compose.podman.yaml`
- Modify: `crates/fernjo-hub/deploy/fernjo-hub.container`
- Modify: `tools/test-release-scripts.sh` (section `# --- hub deploy files ---`)

**Interfaces:**
- Produces: the three files all carry the image reference `ghcr.io/benjo-code/fernjo-hub:0.3.1` on one line (`image: ...` in compose, `Image=...` in the unit); `tools/check-release-version.sh` (Task 5) reads exactly these two lines.

- [ ] **Step 1: Write the failing tests** with `grep -Eq` helpers: `compose_pins_the_image_and_hardens_it` (image line equals the reference above; `user: "10001:10001"`; `init: true`; `read_only: true`; `cap_drop` with `ALL`; `no-new-privileges`; `restart: unless-stopped`; the port mapping `127.0.0.1:8787:8787`; named volume on `/data`), `compose_base_file_has_no_podman_only_key` (`compose.yaml` does not contain `userns_mode` or `keep-id`), `compose_podman_override_holds_keep_id` (`compose.podman.yaml` contains `userns_mode: "keep-id:uid=10001,gid=10001"` and no `image:` line), `quadlet_uses_the_ghcr_image_and_keeps_its_hardening` (no `localhost/fernjo-hub`, the pinned `Image=` line, and the existing `UserNS=keep-id:uid=10001,gid=10001`, `RunInit=true`, `ReadOnly=true`, `DropCapability=ALL`, `NoNewPrivileges=true`, `PublishPort=127.0.0.1:8787:8787` all still present).

- [ ] **Step 2: Run** `bash tools/test-release-scripts.sh` — new cases FAIL.

- [ ] **Step 3: Write `compose.yaml`**: a header comment with the three commands (one-time `init` via `compose run --rm hub init --data /data`, `compose up -d`, `compose down`), `name: fernjo-hub`, service `hub` with exactly the keys of the Global Constraints (`security_opt: ["no-new-privileges:true"]`, `volumes: ["hub-data:/data"]`, top-level `volumes: {hub-data: {}}`); no `command:` (the image's CMD serves on `0.0.0.0:8787`); no `container_name`. `compose run` does not publish service ports, which is why the one-time `init` cannot clash with a running hub. Write `compose.podman.yaml` as the override with only `services.hub.userns_mode`. In both, comment why (Docker rejects the key).

- [ ] **Step 4: Edit `fernjo-hub.container`**: `Image=ghcr.io/benjo-code/fernjo-hub:0.3.1`; update the header comment's `init` pointer to the new `Docs/hub.md` path; leave every other key as it is.

- [ ] **Step 5: Run** `bash tools/test-release-scripts.sh` — PASS. Then, where an engine is available, validate the syntax: `docker compose -f crates/fernjo-hub/deploy/compose.yaml config` exits 0, and `podman compose -f crates/fernjo-hub/deploy/compose.yaml -f crates/fernjo-hub/deploy/compose.podman.yaml config` exits 0; a Quadlet syntax check with `/usr/lib/systemd/system-generators/podman-system-generator --user --dryrun` on a machine that has it. Record which of these could not be run.

- [ ] **Step 6: Commit**

```bash
git add crates/fernjo-hub/deploy tools/test-release-scripts.sh
git commit -m "feat(hub): Compose files and a Quadlet unit for the published image"
```

---

### Task 5: The release version check covers the deploy pins

**Files:**
- Modify: `tools/check-release-version.sh`
- Modify: `tools/test-release-scripts.sh` (the `# --- check-release-version.sh ---` section)

**Interfaces:**
- Consumes: the two lines from Task 4.
- Produces: `check-release-version.sh <tag>` additionally exits 1 with `error: <file> pins image tag '<found>' but the release version is '<want>'` for `crates/fernjo-hub/deploy/compose.yaml` and `crates/fernjo-hub/deploy/fernjo-hub.container` (and an error when the line is missing); a pass prints one line naming all three matches.

- [ ] **Step 1: Make the tests independent of the repo's current version.** The deploy files pin `0.3.1` while `Cargo.toml` still says `0.3.0` between the milestone merge and the release bump, so the existing "accepts the matching tag" tests (`v$cur`) would fail on `dev`. Replace `check_version` with a fixture tree: a function `version_fixture <dir> <cargo ver> <package ver> <compose tag> <quadlet tag>` that creates `<dir>/tools/check-release-version.sh` (copy of the real script; its root is derived from its own location), a `Cargo.toml` with `[workspace.package]` and `version`, a `package.json`, and the two deploy files each with the matching `image:` / `Image=` line. Re-point the existing cases (`..._accepts_the_matching_tag`, `..._without_v`, `..._rejects_a_mismatching_tag`, the two metacharacter cases and the marker check) at a fixture built with all four versions equal to `1.2.3`.

- [ ] **Step 2: Write the new failing tests** against fixtures: `check_release_version_rejects_a_stale_compose_pin` (compose tag `1.2.2`, exit 1, needle `compose.yaml pins image tag '1.2.2'`), `..._rejects_a_stale_quadlet_pin`, `..._rejects_a_missing_pin_line` (the file without an image line), and `..._accepts_when_all_four_agree` (needle `compose.yaml`).

- [ ] **Step 3: Run** `bash tools/test-release-scripts.sh` — the stale/missing cases FAIL.

- [ ] **Step 4: Extend the script**: after the existing two comparisons, read each pin with `sed -n` (first match of `^[[:space:]]*image:[[:space:]]*ghcr\.io/benjo-code/fernjo-hub:\(.*\)$` and `^Image=ghcr\.io/benjo-code/fernjo-hub:\(.*\)$`), strip surrounding quotes and a trailing comment, compare to `$want`, keep the single `status` variable and the existing messages unchanged. A missing file counts as a mismatch with its own message.

- [ ] **Step 5: Run** `bash tools/test-release-scripts.sh` — PASS. Also `bash tools/check-release-version.sh v0.3.1` on the real tree now exits 1 naming `Cargo.toml ... '0.3.0'` (expected until the release bump) and must **not** mention the deploy files (they already say 0.3.1).

- [ ] **Step 6: Commit**

```bash
git add tools/check-release-version.sh tools/test-release-scripts.sh
git commit -m "ci: fail a release whose deploy files pin another hub image tag"
```

---

### Task 6: Release workflow jobs and the dry run

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `tools/test-release-scripts.sh` (section `# --- release.yml: hub image ---`)

**Interfaces:**
- Consumes: `tools/hub-image-tags.sh` (Task 1), the build args (Task 2), `tools/smoke-hub-image.sh` (Task 3).
- Produces: workflow jobs `hub-image-build` and `hub-image`, artifacts named `hub-image-amd64` and `hub-image-arm64` (each holding one file `hub-<arch>.oci.tar`).

- [ ] **Step 1: Write the failing pin tests** (in the style of `publish_goes_through_a_draft`; awk or grep over `release.yml`, each used with `assert_true`):
  - `hub_image_is_the_only_job_with_packages_write`: `packages: write` occurs exactly once and lies between the `  hub-image:` job key and the next job key or end of file.
  - `hub_image_build_has_read_only_permissions`: that job's `permissions:` block is exactly `contents: read`.
  - `hub_image_runs_only_on_a_tag_push`: the `hub-image:` job has `if: github.event_name != 'pull_request' && startsWith(github.ref, 'refs/tags/v')` and `needs: hub-image-build`.
  - `hub_image_build_matrix_has_both_architectures`: the matrix names `ubuntu-latest` and `ubuntu-24.04-arm`.
  - `hub_image_upload_skips_forks`: the upload step of the hub archive carries the same `if:` as the installer upload.
  - `release_pull_request_paths_cover_the_hub_image_inputs`: the `pull_request.paths` list contains `crates/fernjo-hub/Containerfile`, `crates/fernjo-hub/deploy/**` and `.dockerignore`.
  - `hub_image_pushes_with_a_digest_check`: the file contains the step name `Push the manifest list` and `Check that every tag has the same digest`.
  Extend `checkout_refs_are_full_tag_refs` so its expected count of `actions/checkout` refs matches the new total (the installer `build`, `publish`, `hub-image-build`, `hub-image`: 4), keeping the `refs/tags/` assertion.

- [ ] **Step 2: Run** `bash tools/test-release-scripts.sh` — new cases FAIL; `bash tools/check-workflows.sh` still exits 0.

- [ ] **Step 3: Extend the triggers**: add the three paths from the test to `on.pull_request.paths` and say in the header comment that the dry run builds the image for both architectures. Note in a comment that a matching pull request also runs the installer builds, because paths filter the workflow, not a job.

- [ ] **Step 4: Add job `hub-image-build`** (name `hub image (${{ matrix.arch }})`, `timeout-minutes: 45`, `permissions: contents: read`, `fail-fast: false`, matrix `include` with `arch: amd64, os: ubuntu-latest` and `arch: arm64, os: ubuntu-24.04-arm`). Steps, each with `env:` for every non-matrix value:
  1. `actions/checkout` with the same `ref:` expression as the `build` job and `persist-credentials: false`.
  2. When `RELEASE_TAG != ''`: `bash tools/check-release-version.sh "$TAG"`.
  3. `Build the image`: `VERSION="${RELEASE_TAG#v}"` (or `dev` on a pull request or branch run), `REVISION="$GITHUB_SHA"`, then `podman build --build-arg VERSION="$VERSION" --build-arg REVISION="$REVISION" -f crates/fernjo-hub/Containerfile -t localhost/fernjo-hub:test .`. (`GITHUB_SHA` is a predeclared shell variable, not an expression.)
  4. `Smoke-test the image`: `ENGINE=podman bash tools/smoke-hub-image.sh localhost/fernjo-hub:test`.
  5. `Save the image as an OCI archive`: `podman save --format oci-archive -o "hub-${ARCH}.oci.tar" localhost/fernjo-hub:test`, `ARCH` from `matrix.arch` through `env:`.
  6. `actions/upload-artifact` named `hub-image-${{ matrix.arch }}`, `if` identical to the installer upload (not for a fork pull request), `if-no-files-found: error`, `retention-days: 7`.
  If rootless Podman cannot run the smoke test on the runner (the volume ownership copy-up is the likely trouble spot), switch the engine of **both** steps 3-5 to `sudo podman` and say so in a comment; do not weaken the smoke test.

- [ ] **Step 5: Add job `hub-image`**: `needs: hub-image-build`, the `if` of Step 1, `permissions: contents: read`, `packages: write`, `id-token: write`, `attestations: write`, `timeout-minutes: 30`. Steps:
  1. `actions/checkout` at `refs/tags/<tag>` (copy the `publish` job's form).
  2. `actions/download-artifact` with `pattern: hub-image-*` into `hub-archives` (one folder per artifact, no `merge-multiple`).
  3. `Log in to GHCR`: `printf '%s' "$GITHUB_TOKEN" | podman login ghcr.io -u "$GITHUB_ACTOR" --password-stdin` with `GITHUB_TOKEN: ${{ github.token }}` in `env:` (the first push also links the package to this repository through the `source` label).
  4. `Push the manifest list` (step `id: push`): `podman manifest create` a list, `podman manifest add` `oci-archive:hub-archives/hub-image-amd64/hub-amd64.oci.tar` and the arm64 one, then for each reference printed by `bash tools/hub-image-tags.sh "$TAG" ghcr.io/benjo-code/fernjo-hub` run `podman manifest push --all --digestfile "$digestfile" <list> "docker://$ref"`, collecting each digest.
  5. `Check that every tag has the same digest`: fail with `::error::` when the three digests differ or are empty; otherwise write `digest=sha256:...` to `$GITHUB_OUTPUT` (a second `id: digest` step, so the push step stays one concern).
  6. `Verify both architectures`: `skopeo inspect --raw docker://ghcr.io/benjo-code/fernjo-hub:<X.Y.Z>` and assert (with `grep -F`) that `"architecture":"amd64"` and `"architecture":"arm64"` both appear; fail otherwise.
  7. `actions/attest-build-provenance` with `subject-name: ghcr.io/benjo-code/fernjo-hub`, `subject-digest: ${{ steps.digest.outputs.digest }}` (a `with:` value, allowed), `push-to-registry: true`.

- [ ] **Step 6: Run** `bash tools/test-release-scripts.sh` and `bash tools/check-workflows.sh` — both PASS (exit 0, no `file:line` reports).

- [ ] **Step 7: Prove the dry run on GitHub.** Push the branch and open the pull request to `dev` (only on the owner's instruction to push; the plan commits are local). Read the run with `gh run list --branch feature/m12-hub-image` and `gh run view <id>`. Expected: `hub image (amd64)` and `hub image (arm64)` succeed, the smoke test step prints `smoke: ok` in both, `hub-image` is skipped, the `publish` job is skipped, and `gh run view <id> --json jobs` shows no push. Fix the workflow until that holds; record the run id as evidence. Check the fork rule by reading the `if:` of the upload step, not by a fork run.

- [ ] **Step 8: Commit**

```bash
git add .github/workflows/release.yml tools/test-release-scripts.sh
git commit -m "ci: build, smoke-test and publish the hub image from release.yml"
```

---

### Task 7: Documentation, decisions and the release procedure

**Files:**
- Modify: `Docs/hub.md`, `Docs/release-checklist.md` (reusable checklist item 1 and item 9 only), `Docs/DECISIONS.md`, `CLAUDE.md`, `CHANGELOG.md`
- Test: `bash tools/test-release-scripts.sh` and `dotnet run tools/roadmap.cs -- check` stay green

- [ ] **Step 1: `Docs/hub.md`.** Restructure so the pull path comes first: (a) "Set it up from the published image": Compose as the main way (`podman compose -f compose.yaml -f compose.podman.yaml` for rootless Podman, `docker compose -f compose.yaml` for Docker; the `compose run --rm hub init --data /data` one-time step prints the secret once; `up -d`; the `curl` check), then Quadlet (the existing steps with the GHCR image and the pull happening on first start), then plain `podman run` / `docker run` (the existing `init` command with the GHCR reference); (b) "Update to a new hub version" (change the pinned tag in the file you use, `compose pull` / `podman pull` and restart; the volume is kept; the pin is also bumped by every release); (c) "Verify the image" (`gh attestation verify oci://ghcr.io/benjo-code/fernjo-hub:X.Y.Z --repo benjo-code/fernjo`); (d) "Make the package public" as the one-time owner step (GitHub, Packages, `fernjo-hub`, Package settings, Change visibility; the package is private after its first push); (e) "Build the image yourself" keeps today's section as the alternative. Rewrite "Verified so far" and "Not verified yet" truthfully from the evidence collected in Task 8; the images exist from v0.3.1 on, and the doc says so.
- [ ] **Step 2: `Docs/DECISIONS.md`**: one entry "M12: hub image on GHCR" with the choices and why: GHCR (same org, `GITHUB_TOKEN`, attestations), tags `X.Y.Z`/`X.Y`/`latest`, two native runners instead of QEMU, build-test-then-push of the same OCI archive, `packages: write` only in the push job, Podman in CI to match the owner's tooling, no image for v0.3.0, Compose main path and Quadlet supported, deploy-file pins checked by the release script. Record rejected approaches (QEMU emulation; a push per architecture from the build jobs, which would give those jobs `packages: write`).
- [ ] **Step 3: `CLAUDE.md`**: extend "Release workflow" with the two jobs and the hub-image rule; extend "Release scripts" with `hub-image-tags.sh`, `smoke-hub-image.sh` and the new deploy-pin check; change the "Releases" procedure step 1 to include "bump the image tag in `crates/fernjo-hub/deploy/compose.yaml` and `fernjo-hub.container`"; reference by file and name, no line numbers.
- [ ] **Step 4: `Docs/release-checklist.md`**: item 1 names the deploy pins (`check-release-version.sh` now covers them); item 9 names the bump and, after the release, the check `docker buildx imagetools inspect ghcr.io/benjo-code/fernjo-hub:X.Y.Z` and `gh attestation verify oci://...`.
- [ ] **Step 5: `CHANGELOG.md`** under `[Unreleased]`: a short, user-readable entry: the hub now comes as a ready-made image (`ghcr.io/benjo-code/fernjo-hub`, amd64 and arm64) with a Compose file and an updated Quadlet unit; no build from source needed; the app is unchanged. Name milestone 12.
- [ ] **Step 6: Verify and commit.** `bash tools/test-release-scripts.sh` and `dotnet run tools/roadmap.cs -- check` pass.

```bash
git add Docs/hub.md Docs/release-checklist.md Docs/DECISIONS.md CLAUDE.md CHANGELOG.md
git commit -m "docs: publish the hub image, Compose and Quadlet paths (M12)"
```

---

### Task 8: Roadmap, evidence and the gate

**Files:**
- Modify: `Docs/roadmap.json`, then regenerate `Docs/ROADMAP.md`

- [ ] **Step 1: Tick the tasks** in `Docs/roadmap.json` as each of Tasks 1 to 7 lands (`"done": true`). Tick an acceptance criterion only with its proof in `"evidence"`:
  - Build, test and lint green: the exact commands from the roadmap and their results (`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `bash tools/check-workflows.sh`, `bash tools/test-release-scripts.sh`, `dotnet run tools/roadmap.cs -- check`).
  - The pull request dry run: the run id from Task 6 Step 7.
  - Left open (not ticked) with the verifying command in the text: the post-tag manifest check, `gh attestation verify`, the two owner Podman checks, the public-package check and the Docker Compose check where Docker is unavailable. They can only be run after the `v0.3.1` tag, which is a separate release step on the owner's instruction.
- [ ] **Step 2: Set status `done` only after the owner's checks**; until then keep `in-progress` and say what is open under "Scope changes" if the plan changed (for example the `sudo podman` fallback of Task 6).
- [ ] **Step 3: Run the gate** and record the results: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (the SSH integration tests skip without the container; say so), `bash tools/check-workflows.sh`, `bash tools/test-release-scripts.sh`, `dotnet run tools/roadmap.cs -- generate` then `-- check`.
- [ ] **Step 4: Commit**

```bash
git add Docs/roadmap.json Docs/ROADMAP.md
git commit -m "docs: tick off milestone 12 tasks and record the evidence"
```

---

## Self-review (against the roadmap's tasks and criteria)

- Workflow job and tags: Tasks 1, 6. Dry run on a pull request: Task 6 (paths, no push, read-only). Smoke before push: Tasks 3 and 6 (the job that pushes `needs` the job that tests). Image metadata: Task 2. Compose and the Podman override: Task 4. Quadlet: Task 4. `Docs/hub.md`: Task 7. Release scripts, checklist and `CLAUDE.md` procedure: Tasks 5 and 7. Decisions, facts and changelog: Task 7. Roadmap and evidence: Task 8.
- Acceptance criteria that need the tag, GHCR visibility or the owner's machines are named in Task 8 as open with their commands, not claimed.
- Names used across tasks: `hub-image-build`, `hub-image`, `hub-image-amd64`/`hub-image-arm64`, `hub-<arch>.oci.tar`, `tools/hub-image-tags.sh <tag> <image>`, `tools/smoke-hub-image.sh <image>` with `ENGINE`, build args `VERSION` and `REVISION`, the pin lines `image: ghcr.io/benjo-code/fernjo-hub:0.3.1` and `Image=ghcr.io/benjo-code/fernjo-hub:0.3.1`.

**Known risks, to be resolved by the dry run, not by guessing:** rootless Podman on the runners (volume ownership; fallback `sudo podman`), `ubuntu-24.04-arm` availability for this repository, and the `podman manifest add oci-archive:` plus `skopeo` versions on `ubuntu-latest`.

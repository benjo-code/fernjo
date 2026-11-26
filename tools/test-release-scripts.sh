#!/usr/bin/env bash
# Tests for tools/check-workflows.sh, check-release-version.sh, hub-image-tags.sh, verify-installers.sh and third-party-notices.mjs.
# Plain bash, temp dirs, no dependencies. Run: bash tools/test-release-scripts.sh
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

failures=0
total=0

# assert_exit <name> <expected exit> <expected output substring or ""> <command...>
assert_exit() {
  local name="$1" want="$2" needle="$3"
  shift 3
  total=$((total + 1))
  local out got
  out="$("$@" 2>&1)"
  got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL $name: exit $got, expected $want"
    echo "$out" | sed 's/^/     | /'
    failures=$((failures + 1))
    return
  fi
  if [ -n "$needle" ] && [[ "$out" != *"$needle"* ]]; then
    echo "FAIL $name: output lacks '$needle'"
    echo "$out" | sed 's/^/     | /'
    failures=$((failures + 1))
    return
  fi
  echo "ok   $name"
}

sha=0123456789abcdef0123456789abcdef01234567

# A workflow that satisfies every rule; bad fixtures are derived from it.
good_workflow() {
  cat <<YML
name: T
on: push
permissions:
  contents: read
jobs:
  j:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@$sha # v1
      - uses: ./local-action
      - name: step
        env:
          TAG: \${{ github.event.release.tag_name }}
        run: |
          echo "\$TAG"
YML
}

# bad_fixture <name> <sed expression applied to the good workflow>
bad_fixture() {
  local d="$tmp/wf-$1"
  mkdir -p "$d"
  good_workflow | sed -e "$2" >"$d/w.yml"
  echo "$d"
}

# --- check-workflows.sh ---
check_workflows="$root/tools/check-workflows.sh"

assert_exit check_workflows_passes_on_the_real_workflows 0 "" bash "$check_workflows"

d="$tmp/wf-good"
mkdir -p "$d"
good_workflow >"$d/w.yml"
assert_exit check_workflows_passes_on_the_good_fixture 0 "" bash "$check_workflows" "$d"

d="$(bad_fixture unpinned "s|actions/checkout@$sha # v1|actions/checkout@v4|")"
assert_exit check_workflows_rejects_an_unpinned_action 1 "w.yml:9: " bash "$check_workflows" "$d"
assert_exit check_workflows_names_the_reason_for_an_unpinned_action 1 "commit SHA" bash "$check_workflows" "$d"

d="$(bad_fixture selfhosted "s|runs-on: ubuntu-latest|runs-on: [self-hosted, linux]|")"
assert_exit check_workflows_rejects_self_hosted 1 "self-hosted" bash "$check_workflows" "$d"

d="$(bad_fixture prt "s|on: push|on: pull_request_target|")"
assert_exit check_workflows_rejects_pull_request_target 1 "pull_request_target" bash "$check_workflows" "$d"

d="$(bad_fixture noperm "/^permissions:/,/^  contents: read/d")"
assert_exit check_workflows_rejects_missing_permissions 1 "permissions" bash "$check_workflows" "$d"

expr_event='s|echo "\$TAG"|echo "${{ github.event.release.tag_name }}"|'
d="$(bad_fixture inline "$expr_event")"
assert_exit check_workflows_rejects_inline_github_event_in_run 1 "github.event" bash "$check_workflows" "$d"

expr_input='s|echo "\$TAG"|echo "${{ inputs.tag }}"|'
d="$(bad_fixture inlineinput "$expr_input")"
assert_exit check_workflows_rejects_inline_inputs_in_run 1 "inputs" bash "$check_workflows" "$d"

# Every form below must be rejected inside run:, whatever the expression starts with.
run_expr_case() { # <name> <expression>
  local e="$2"
  local d="$tmp/wf-expr-$1"
  mkdir -p "$d"
  good_workflow | awk -v e="$e" '/echo "\$TAG"/ { sub(/echo "\$TAG"/, "echo \"${{ " e " }}\"") } { print }' >"$d/w.yml"
  assert_exit "check_workflows_rejects_run_expression_$1" 1 "not allowed inside run:" bash "$check_workflows" "$d"
}
run_expr_case head_ref 'github.head_ref'
run_expr_case ref_name 'github.ref_name'
run_expr_case ref 'github.ref'
run_expr_case base_ref 'github.base_ref'
run_expr_case tojson_event 'toJSON(github.event)'
run_expr_case bracket "github['event']['release']['tag_name']"
run_expr_case needs_output 'needs.x.outputs.y'
run_expr_case steps_output 'steps.x.outputs.y'
run_expr_case nested_event 'format(x, github.event.release.body)'

d="$tmp/wf-expr-allowed"
mkdir -p "$d"
good_workflow | awk '/echo "\$TAG"/ { print "          echo \"${{ matrix.os }} ${{ runner.os }} ${{ env.X }} ${{ secrets.S }} ${{ github.event_name }} ${{ github.sha }} ${{ github.repository }} ${{ github.workspace }}\""; next } { print }' >"$d/w.yml"
assert_exit check_workflows_allows_safe_run_expressions 0 "" bash "$check_workflows" "$d"

# A } inside a quoted literal must not hide the expression; unterminated expressions fail closed.
run_expr_case format_literal_brace "format('{0}', github.ref_name)"
run_expr_case format_literal_brace_text "format('x{0}y', github.ref_name)"
run_expr_case disallowed_function "toJSON(matrix)"
run_expr_case index_after_space "github ['event']"

d="$tmp/wf-expr-unterminated"
mkdir -p "$d"
good_workflow | awk '/echo "\$TAG"/ { print "          echo ${{ github.head_ref"; next } { print }' >"$d/w.yml"
assert_exit check_workflows_rejects_an_unterminated_run_expression 1 "not closed on this line" bash "$check_workflows" "$d"

d="$tmp/wf-expr-allowed-literal"
mkdir -p "$d"
good_workflow | awk '/echo "\$TAG"/ { print "          echo \"${{ format('"'"'{0}-{1}'"'"', matrix.os, runner.arch) }} ${{ contains(matrix.os, '"'"'win}}'"'"') }} ${{ startsWith(runner.os, '"'"'L'"'"') }}\""; next } { print }' >"$d/w.yml"
assert_exit check_workflows_allows_functions_and_literals_over_allowed_identifiers 0 "" bash "$check_workflows" "$d"

# YAML forms that used to slip past the line-based checks (M11 review #69). Each must fail.
# wf_case <name> <expected output substring> <workflow body after "jobs:"> [<"on:" value>]
wf_case() {
  local d="$tmp/wf-form-$1"
  mkdir -p "$d"
  printf 'name: T\non: %s\npermissions:\n  contents: read\njobs:\n%s\n' "${4:-push}" "$3" >"$d/w.yml"
  assert_exit "check_workflows_rejects_form_$1" 1 "$2" bash "$check_workflows" "$d"
}
job='  j:
    runs-on: ubuntu-latest
    steps:'
e='${{ github.event.release.body }}'
wf_case comment_after_block_indicator "not allowed inside run:" "$job
      - name: s
        run: | # note
          echo \"$e\""
wf_case comment_line_inside_run_block "not allowed inside run:" "$job
      - name: s
        run: |
          # note $e
          echo ok"
wf_case flow_step_with_run "flow" "$job
      - { name: s, run: \"echo $e\" }"
wf_case flow_step_with_unpinned_uses "flow" "$job
      - { uses: actions/checkout@v4 }"
wf_case flow_steps_sequence "flow" "  j:
    runs-on: ubuntu-latest
    steps: [ { uses: actions/checkout@v4 } ]"
wf_case quoted_run_key "quoted" "$job
      - name: s
        \"run\": echo $e"
wf_case quoted_unpinned_uses_key "quoted" "$job
      - \"uses\": actions/checkout@v4"
wf_case spaced_run_key "run" "$job
      - name: s
        run : echo $e"
wf_case multi_line_quoted_run_value "not allowed inside run:" "$job
      - name: s
        run: \"echo a
          $e\""
wf_case multi_line_plain_run_value "not allowed inside run:" "$job
      - name: s
        run: echo \"a
          $e\""
wf_case open_quote_run_value "closed" "$job
      - name: s
        run: 'echo a
      $e'"
wf_case github_script_script "not allowed inside" "$job
      - uses: actions/github-script@$sha # v7
        with:
          script: |
            console.log(\"$e\")"
wf_case yaml_alias "alias" "$job
      - &step
        name: s
        run: echo ok
      - *step"
wf_case explicit_key "explicit" "$job
      - name: s
        ? run
        : echo $e"
wf_case explicit_key_in_sequence_item "explicit" "$job
      - ? uses
        : actions/checkout@v4"
wf_case escaped_quoted_key "backslash" "$job
      - name: s
        \"r\\u0075n\": echo $e"
wf_case workflow_run_trigger "workflow_run" "$job
      - run: echo ok" "workflow_run"

# The hardened checks must still accept the ordinary forms.
d="$tmp/wf-form-ordinary"
mkdir -p "$d"
cat >"$d/w.yml" <<YML
name: T
on: push
permissions:
  contents: read
defaults:
  run:
    shell: bash
jobs:
  j:
    runs-on: ubuntu-latest
    steps:
      - name: s
        run: | # a YAML comment
          # a shell comment naming \${{ matrix.os }}
          echo a && echo b
      - run: 'echo it''s ok \${{ runner.os }}' # trailing comment
      - run: echo "plain \${{ env.X }}"
      - uses: actions/github-script@$sha # v7
        with:
          script: |
            console.log("\${{ github.sha }}")
YML
assert_exit check_workflows_accepts_the_ordinary_block_forms 0 "" bash "$check_workflows" "$d"

# --- check-release-version.sh ---
# The real tree pins the hub image to the next release while Cargo.toml still has the current one
# (between the milestone merge and the release bump), so the cases run on fixture trees. The script
# finds its root from its own location, so each fixture holds a copy of it.
# version_fixture <dir> <cargo ver> <package ver> <compose tag> <quadlet tag>
version_fixture() {
  local d="$1"
  mkdir -p "$d/tools" "$d/crates/fernjo-hub/deploy"
  cp "$root/tools/check-release-version.sh" "$d/tools/"
  printf '[workspace.package]\nversion = "%s"\n' "$2" >"$d/Cargo.toml"
  printf '{\n  "name": "x",\n  "version": "%s"\n}\n' "$3" >"$d/package.json"
  printf 'services:\n  hub:\n    image: ghcr.io/benjo-code/fernjo-hub:%s\n' "$4" >"$d/crates/fernjo-hub/deploy/compose.yaml"
  printf '[Container]\nImage=ghcr.io/benjo-code/fernjo-hub:%s\n' "$5" >"$d/crates/fernjo-hub/deploy/fernjo-hub.container"
}
version_fixture "$tmp/ver-ok" 1.2.3 1.2.3 1.2.3 1.2.3
check_version="$tmp/ver-ok/tools/check-release-version.sh"

assert_exit check_release_version_accepts_the_matching_tag 0 "" bash "$check_version" "v1.2.3"
assert_exit check_release_version_accepts_the_tag_without_v 0 "" bash "$check_version" "1.2.3"
assert_exit check_release_version_accepts_when_all_four_agree 0 "compose.yaml" bash "$check_version" "v1.2.3"
assert_exit check_release_version_rejects_a_mismatching_tag 1 "does not match" bash "$check_version" "v99.99.99"
version_fixture "$tmp/ver-compose" 1.2.3 1.2.3 1.2.2 1.2.3
assert_exit check_release_version_rejects_a_stale_compose_pin 1 "compose.yaml pins image tag '1.2.2'" \
  bash "$tmp/ver-compose/tools/check-release-version.sh" v1.2.3
version_fixture "$tmp/ver-quadlet" 1.2.3 1.2.3 1.2.3 1.2.2
assert_exit check_release_version_rejects_a_stale_quadlet_pin 1 "fernjo-hub.container pins image tag '1.2.2'" \
  bash "$tmp/ver-quadlet/tools/check-release-version.sh" v1.2.3
version_fixture "$tmp/ver-missing" 1.2.3 1.2.3 1.2.3 1.2.3
printf 'services:\n  hub: {}\n' >"$tmp/ver-missing/crates/fernjo-hub/deploy/compose.yaml"
assert_exit check_release_version_rejects_a_missing_pin_line 1 "compose.yaml has no hub image line" \
  bash "$tmp/ver-missing/tools/check-release-version.sh" v1.2.3
version_fixture "$tmp/ver-nofile" 1.2.3 1.2.3 1.2.3 1.2.3
rm "$tmp/ver-nofile/crates/fernjo-hub/deploy/fernjo-hub.container"
assert_exit check_release_version_rejects_a_missing_deploy_file 1 "fernjo-hub.container is missing" \
  bash "$tmp/ver-nofile/tools/check-release-version.sh" v1.2.3
version_fixture "$tmp/ver-cargo" 1.2.2 1.2.3 1.2.3 1.2.3
assert_exit check_release_version_still_rejects_a_stale_cargo_version 1 "Cargo.toml" \
  bash "$tmp/ver-cargo/tools/check-release-version.sh" v1.2.3

marker="$tmp/pwned"
assert_exit check_release_version_rejects_a_tag_with_shell_metacharacters 1 "not a valid" \
  bash "$check_version" "v1.0.0; touch $marker"
assert_exit check_release_version_rejects_command_substitution 1 "not a valid" \
  bash "$check_version" '$(touch '"$marker"')'
total=$((total + 1))
if [ -e "$marker" ]; then
  echo "FAIL check_release_version_does_not_evaluate_the_tag: marker file exists"
  failures=$((failures + 1))
else
  echo "ok   check_release_version_does_not_evaluate_the_tag"
fi

# --- hub-image-tags.sh ---
hub_tags="$root/tools/hub-image-tags.sh"

# assert_lines <name> <expected output, one line each> <command...>: exit 0 and the output equals the lines exactly.
assert_lines() {
  local name="$1" want="$2"
  shift 2
  total=$((total + 1))
  local out got
  out="$("$@" 2>&1)"
  got=$?
  if [ "$got" -ne 0 ] || [ "$out" != "$want" ]; then
    echo "FAIL $name: exit $got, output:"
    echo "$out" | sed 's/^/     | /'
    failures=$((failures + 1))
    return
  fi
  echo "ok   $name"
}

want_tags=$'ghcr.io/x/y:0.3.1\nghcr.io/x/y:0.3\nghcr.io/x/y:latest'
assert_lines hub_image_tags_for_a_plain_version "$want_tags" bash "$hub_tags" v0.3.1 ghcr.io/x/y
assert_lines hub_image_tags_accept_the_tag_without_v "$want_tags" bash "$hub_tags" 0.3.1 ghcr.io/x/y
# A stray tag must produce no image tags at all, so it can never move `latest`.
assert_exit hub_image_tags_reject_a_prerelease 1 "not a valid" bash "$hub_tags" v0.3.1-rc.1 ghcr.io/x/y
assert_exit hub_image_tags_reject_two_parts 1 "not a valid" bash "$hub_tags" v1.2 ghcr.io/x/y
assert_exit hub_image_tags_reject_four_parts 1 "not a valid" bash "$hub_tags" v1.2.3.4 ghcr.io/x/y
hub_marker="$tmp/hub-pwned"
assert_exit hub_image_tags_reject_shell_metacharacters 1 "not a valid" \
  bash "$hub_tags" "v1.2.3; touch $hub_marker" ghcr.io/x/y
assert_exit hub_image_tags_reject_command_substitution 1 "not a valid" \
  bash "$hub_tags" '$(touch '"$hub_marker"')' ghcr.io/x/y
total=$((total + 1))
if [ -e "$hub_marker" ]; then
  echo "FAIL hub_image_tags_do_not_evaluate_the_tag: marker file exists"
  failures=$((failures + 1))
else
  echo "ok   hub_image_tags_do_not_evaluate_the_tag"
fi
assert_exit hub_image_tags_wrong_argument_count 2 "usage" bash "$hub_tags" v0.3.1
assert_exit hub_image_tags_no_arguments 2 "usage" bash "$hub_tags"

# --- verify-installers.sh ---
verify="$root/tools/verify-installers.sh"
ver=1.2.3
names=(
  "fernjo_${ver}_x64-setup.exe"
  "fernjo_${ver}_aarch64.dmg"
  "fernjo_${ver}_x64.dmg"
  "fernjo_${ver}_amd64.AppImage"
  "fernjo_${ver}_amd64.deb"
)

d="$tmp/inst-ok"
mkdir -p "$d"
for n in "${names[@]}"; do : >"$d/$n"; done
assert_exit verify_installers_passes_on_the_complete_set 0 "" bash "$verify" "$ver" "$d"

d="$tmp/inst-missing"
mkdir -p "$d"
for n in "${names[@]}"; do : >"$d/$n"; done
rm "$d/fernjo_${ver}_x64.dmg"
assert_exit verify_installers_fails_naming_the_missing_file 1 "missing: fernjo_${ver}_x64.dmg" bash "$verify" "$ver" "$d"

d="$tmp/inst-extra"
mkdir -p "$d"
for n in "${names[@]}"; do : >"$d/$n"; done
: >"$d/surprise.zip"
assert_exit verify_installers_fails_on_an_unexpected_extra_file 1 "unexpected: surprise.zip" bash "$verify" "$ver" "$d"

# --artifacts: one folder per build job's artifact; nothing may cross platforms (M11 review #72).
# artifacts_fixture <name>: the layout publish downloads when every job uploaded only its own files.
artifacts_fixture() {
  local a="$tmp/art-$1"
  mkdir -p "$a/installers-windows" "$a/installers-macos-arm64" "$a/installers-macos-x64" "$a/installers-linux"
  echo win >"$a/installers-windows/${names[0]}"
  echo arm >"$a/installers-macos-arm64/${names[1]}"
  echo x64 >"$a/installers-macos-x64/${names[2]}"
  echo img >"$a/installers-linux/${names[3]}"
  echo deb >"$a/installers-linux/${names[4]}"
  echo "$a"
}

a="$(artifacts_fixture ok)"
assert_exit verify_installers_artifacts_pass_and_collect_the_set 0 "all 5 installers present" \
  bash "$verify" "$ver" --artifacts "$a" "$a.out"
total=$((total + 1))
if [ "$(cat "$a.out/${names[0]}" 2>/dev/null)" = win ] && [ "$(ls "$a.out" 2>/dev/null | wc -l)" -eq 5 ]; then
  echo "ok   verify_installers_artifacts_copy_each_platform_file_unchanged"
else
  echo "FAIL verify_installers_artifacts_copy_each_platform_file_unchanged"
  failures=$((failures + 1))
fi

a="$(artifacts_fixture cross)"
echo evil >"$a/installers-linux/${names[0]}"
assert_exit verify_installers_artifacts_reject_another_platforms_installer 1 \
  "unexpected: installers-linux/${names[0]}" bash "$verify" "$ver" --artifacts "$a" "$a.out"
total=$((total + 1))
if [ -e "$a.out" ]; then
  echo "FAIL verify_installers_artifacts_collect_nothing_on_a_mismatch: $a.out exists"
  failures=$((failures + 1))
else
  echo "ok   verify_installers_artifacts_collect_nothing_on_a_mismatch"
fi

a="$(artifacts_fixture extra-artifact)"
mkdir -p "$a/installers-zz"
echo evil >"$a/installers-zz/${names[0]}"
assert_exit verify_installers_artifacts_reject_an_extra_artifact 1 "unexpected: art-extra-artifact/installers-zz" \
  bash "$verify" "$ver" --artifacts "$a" "$a.out"

a="$(artifacts_fixture missing-platform)"
rm -r "$a/installers-macos-x64"
assert_exit verify_installers_artifacts_reject_a_missing_platform 1 "missing: art-missing-platform/installers-macos-x64" \
  bash "$verify" "$ver" --artifacts "$a" "$a.out"

a="$(artifacts_fixture extra-file)"
: >"$a/installers-windows/readme.txt"
assert_exit verify_installers_artifacts_reject_an_extra_file_in_a_platform 1 "unexpected: installers-windows/readme.txt" \
  bash "$verify" "$ver" --artifacts "$a" "$a.out"

# The old merged layout (download-artifact with merge-multiple) must not pass.
a="$tmp/art-flat"
mkdir -p "$a"
for n in "${names[@]}"; do : >"$a/$n"; done
assert_exit verify_installers_artifacts_reject_a_merged_folder 1 "unexpected: art-flat/${names[0]}" \
  bash "$verify" "$ver" --artifacts "$a" "$a.out"

a="$(artifacts_fixture out-exists)"
mkdir -p "$a.out"
assert_exit verify_installers_artifacts_refuse_an_existing_out_dir 1 "already exists" \
  bash "$verify" "$ver" --artifacts "$a" "$a.out"

# --- release.yml: pins for the M11 review fixes #70, #72 and #73 (the behaviour needs a real run) ---
release_yml="$root/.github/workflows/release.yml"
# assert_true <name> <command...>
assert_true() {
  local name="$1"
  shift
  total=$((total + 1))
  if "$@"; then echo "ok   $name"; else echo "FAIL $name"; failures=$((failures + 1)); fi
}
checkout_refs_are_full_tag_refs() {
  local refs
  refs="$(awk '/uses: actions\/checkout@/ { c = 1; next } c && /^ *ref:/ { print; c = 0 } /^ *- / { c = 0 }' "$release_yml")"
  [ "$(printf '%s\n' "$refs" | grep -c .)" -eq 4 ] && ! printf '%s\n' "$refs" | grep -vq "refs/tags/"
}
assert_true release_checkouts_use_full_tag_refs checkout_refs_are_full_tag_refs
upload_skips_forks() {
  awk '/uses: actions\/upload-artifact@/ { getline; print; exit }' "$release_yml" |
    grep -Fq "if: github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository"
}
assert_true release_upload_skips_fork_pull_requests upload_skips_forks
publish_keeps_artifacts_apart() {
  ! grep -q "merge-multiple:" "$release_yml" && grep -Fq 'verify-installers.sh "${TAG#v}" --artifacts artifacts installers' "$release_yml"
}
assert_true release_publish_keeps_each_artifact_in_its_own_folder publish_keeps_artifacts_apart
# Immutable releases (M11 review #71): the tag push starts the run, never a published release (which
# takes no new files), and publish checks for a draft first, attaches, then publishes last.
publish_goes_through_a_draft() {
  local steps
  steps="$(grep -E '^ *- name: (Check that the release is a draft|Attach the installers to the draft|Publish the release)$' "$release_yml" | sed 's/^ *- name: //')"
  ! grep -Eq '^  release:' "$release_yml" &&
    grep -Fxq "    if: github.event_name != 'pull_request' && startsWith(github.ref, 'refs/tags/v')" "$release_yml" &&
    awk '/^on:/ { o = 1; next } o && /^[^ ]/ { o = 0 } o && /^  push:/ { p = 1; next } p && /^    tags:/ { t = 1; next } t && /^      - .v\*.$/ { f = 1 } END { exit !f }' "$release_yml" &&
    [ "$steps" = "$(printf '%s\n' 'Check that the release is a draft' 'Attach the installers to the draft' 'Publish the release')" ] &&
    [ "$(grep -E '^ *- name: ' "$release_yml" | tail -n 1 | sed 's/^ *- name: //')" = 'Publish the release' ]
}
assert_true release_publish_attaches_to_a_draft_then_publishes publish_goes_through_a_draft

# --- release.yml: hub image ---
# job_block <job key>: the lines of one job of release.yml (from its key to the next job key).
job_block() {
  awk -v job="  $1:" '$0 == job { p = 1; print; next } p && /^  [a-z][a-z0-9-]*:$/ { exit } p { print }' "$release_yml"
}
hub_image_is_the_only_job_with_packages_write() {
  [ "$(grep -c 'packages: write' "$release_yml")" -eq 1 ] &&
    job_block hub-image | grep -Fxq '      packages: write'
}
assert_true release_hub_image_is_the_only_job_with_packages_write hub_image_is_the_only_job_with_packages_write
hub_image_build_has_read_only_permissions() {
  local perms
  perms="$(job_block hub-image-build | awk '/^    permissions:$/ { p = 1; next } p && /^      [a-z-]+:/ { print; next } p { exit }')"
  [ "$perms" = '      contents: read' ]
}
assert_true release_hub_image_build_has_read_only_permissions hub_image_build_has_read_only_permissions
hub_image_runs_only_on_a_tag_push() {
  local block
  block="$(job_block hub-image)"
  printf '%s\n' "$block" | grep -Fxq "    if: github.event_name != 'pull_request' && startsWith(github.ref, 'refs/tags/v')" &&
    printf '%s\n' "$block" | grep -Fxq '    needs: hub-image-build'
}
assert_true release_hub_image_runs_only_on_a_tag_push hub_image_runs_only_on_a_tag_push
hub_image_build_matrix_has_both_architectures() {
  local block
  block="$(job_block hub-image-build)"
  printf '%s\n' "$block" | grep -Fq 'os: ubuntu-latest' && printf '%s\n' "$block" | grep -Fq 'os: ubuntu-24.04-arm'
}
assert_true release_hub_image_build_matrix_has_both_architectures hub_image_build_matrix_has_both_architectures
hub_image_upload_skips_forks() {
  job_block hub-image-build | awk '/uses: actions\/upload-artifact@/ { getline; print; exit }' |
    grep -Fq "if: github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository"
}
assert_true release_hub_image_upload_skips_fork_pull_requests hub_image_upload_skips_forks
pull_request_paths_cover_the_hub_image() {
  local paths want
  paths="$(awk '/^on:/ { o = 1; next } o && /^[^ ]/ { o = 0 } o && /^  pull_request:/ { p = 1; next } p && /^  [a-z_]+:/ { p = 0 } p && /^      - / { print }' "$release_yml")"
  for want in crates/fernjo-hub/Containerfile 'crates/fernjo-hub/deploy/**' .dockerignore; do
    printf '%s\n' "$paths" | grep -Fxq "      - $want" || return 1
  done
}
assert_true release_pull_request_paths_cover_the_hub_image_inputs pull_request_paths_cover_the_hub_image
hub_image_pushes_with_a_digest_check() {
  job_block hub-image | grep -Fxq '      - name: Push the manifest list' &&
    job_block hub-image | grep -Fxq '      - name: Check that every tag has the same digest'
}
assert_true release_hub_image_pushes_with_a_digest_check hub_image_pushes_with_a_digest_check
# The installer release must not be published while the image it points at is missing: the published
# release is immutable and its deploy files pin that image.
publish_waits_for_the_hub_image() {
  job_block publish | awk '/^    needs:$/ { n = 1; next } n && /^      - / { sub(/^ +/, ""); print; next } n { exit }' | sort | tr '\n' ' ' |
    grep -Fxq -- '- build - hub-image '
}
assert_true release_publish_waits_for_the_hub_image publish_waits_for_the_hub_image
hub_image_checks_the_registry_digests() {
  job_block hub-image | grep -Fq "skopeo inspect --format '{{.Digest}}'"
}
assert_true release_hub_image_checks_the_registry_digests hub_image_checks_the_registry_digests
hub_image_annotates_the_index_with_the_source() {
  job_block hub-image | grep -Fq 'podman manifest annotate --index --annotation org.opencontainers.image.source=https://github.com/benjo-code/fernjo'
}
assert_true release_hub_image_annotates_the_index_with_the_source hub_image_annotates_the_index_with_the_source

# --- hub Containerfile ---
hub_containerfile="$root/crates/fernjo-hub/Containerfile"
containerfile_final_stage_is_scratch_as_10001() {
  grep -Fxq 'FROM scratch' "$hub_containerfile" &&
    grep -Fxq 'USER 10001:10001' "$hub_containerfile" &&
    grep -Fxq 'VOLUME /data' "$hub_containerfile" &&
    grep -Fxq 'COPY --from=build --chown=10001:10001 /data /data' "$hub_containerfile"
}
assert_true hub_containerfile_final_stage_is_scratch_as_10001 containerfile_final_stage_is_scratch_as_10001
containerfile_has_the_oci_labels() {
  local key
  for key in source version revision licenses description; do
    grep -Fq "org.opencontainers.image.$key=" "$hub_containerfile" || return 1
  done
  grep -Fq 'org.opencontainers.image.licenses="MIT OR Apache-2.0"' "$hub_containerfile" &&
    grep -Fq 'org.opencontainers.image.source="https://github.com/benjo-code/fernjo"' "$hub_containerfile" &&
    grep -Fxq 'ARG VERSION=dev' "$hub_containerfile" &&
    grep -Fxq 'ARG REVISION=unknown' "$hub_containerfile"
}
assert_true hub_containerfile_has_the_oci_labels containerfile_has_the_oci_labels

# --- smoke-hub-image.sh ---
smoke_hub="$root/tools/smoke-hub-image.sh"
assert_exit smoke_hub_image_needs_an_image 2 "usage" bash "$smoke_hub"
assert_exit smoke_hub_image_refuses_an_unknown_engine 2 "ENGINE" env ENGINE=rm bash "$smoke_hub" some/image:tag

# A stub engine first on PATH logs every call; `run` behaves as $STUB_RUN says. The script's
# trap must still remove the volume and the container it named.
fake_engine() {
  local bin="$tmp/fake-bin-$1"
  mkdir -p "$bin"
  cat >"$bin/docker" <<STUB
#!/usr/bin/env bash
echo "\$*" >>"$tmp/fake-$1.log"
case "\$1" in
  run) $2 ;;
esac
exit 0
STUB
  chmod +x "$bin/docker"
  echo "$bin"
}
fake_bin="$(fake_engine fails 'exit 1')"
assert_exit smoke_hub_image_fails_naming_the_step_with_a_fake_engine 1 "FAILED at step: init" \
  env PATH="$fake_bin:$PATH" bash "$smoke_hub" some/image:tag
smoke_cleans_up_what_it_created() {
  local log="$tmp/fake-fails.log" vol
  vol="$(sed -n 's/^volume create \(fernjo-hub-smoke-.*\)$/\1/p' "$log" | head -n 1)"
  [ -n "$vol" ] &&
    grep -Fxq "volume rm -f $vol" "$log" &&
    grep -Fxq "rm -f $vol" "$log"
}
assert_true smoke_hub_image_cleans_up_the_volume_and_container_after_a_failure smoke_cleans_up_what_it_created

# A hanging engine call is cut off by the timeout, and the cleanup still runs.
hang_bin="$(fake_engine hangs 'exec sleep 30')"
assert_exit smoke_hub_image_times_out_a_hanging_engine_call 1 "FAILED at step: init" \
  env PATH="$hang_bin:$PATH" SMOKE_TIMEOUT=1 bash "$smoke_hub" some/image:tag
smoke_cleans_up_after_a_hang() {
  grep -Eq '^volume rm -f fernjo-hub-smoke-' "$tmp/fake-hangs.log" &&
    grep -Eq '^rm -f fernjo-hub-smoke-' "$tmp/fake-hangs.log"
}
assert_true smoke_hub_image_cleans_up_after_a_hang smoke_cleans_up_after_a_hang

# --- hub deploy files ---
deploy="$root/crates/fernjo-hub/deploy"
hub_ref='ghcr.io/benjo-code/fernjo-hub:0.3.1'
compose_pins_the_image_and_hardens_it() {
  local f="$deploy/compose.yaml"
  grep -Eq "^    image: $hub_ref\$" "$f" &&
    grep -Eq '^    user: "10001:10001"$' "$f" &&
    grep -Eq '^    init: true$' "$f" &&
    grep -Eq '^    read_only: true$' "$f" &&
    grep -Eq '^    cap_drop: \[ALL\]$' "$f" &&
    grep -Eq '^    security_opt: \["no-new-privileges:true"\]$' "$f" &&
    grep -Eq '^    restart: unless-stopped$' "$f" &&
    grep -Eq '^      - "127\.0\.0\.1:8787:8787"$' "$f" &&
    grep -Eq '^      - "hub-data:/data"$' "$f" &&
    grep -Eq '^  hub-data: \{\}$' "$f"
}
assert_true hub_compose_pins_the_image_and_hardens_it compose_pins_the_image_and_hardens_it
compose_base_file_has_no_podman_only_key() {
  [ -f "$deploy/compose.yaml" ] && ! grep -Eq '^[^#]*(userns_mode|keep-id)' "$deploy/compose.yaml"
}
assert_true hub_compose_base_file_has_no_podman_only_key compose_base_file_has_no_podman_only_key
compose_podman_override_holds_keep_id() {
  local f="$deploy/compose.podman.yaml"
  grep -Fxq '    userns_mode: "keep-id:uid=10001,gid=10001"' "$f" && ! grep -Eq '^[^#]*image:' "$f"
}
assert_true hub_compose_podman_override_holds_keep_id compose_podman_override_holds_keep_id
compose_podman_override_turns_the_pod_off() {
  # podman-compose refuses --userns together with its default pod
  grep -Fxq 'x-podman:' "$deploy/compose.podman.yaml" && grep -Fxq '  in_pod: false' "$deploy/compose.podman.yaml"
}
assert_true hub_compose_podman_override_turns_the_pod_off compose_podman_override_turns_the_pod_off
quadlet_uses_the_ghcr_image_and_keeps_its_hardening() {
  local f="$deploy/fernjo-hub.container" line
  ! grep -Fq 'localhost/fernjo-hub' "$f" || return 1
  for line in "Image=$hub_ref" 'UserNS=keep-id:uid=10001,gid=10001' 'RunInit=true' 'ReadOnly=true' \
    'DropCapability=ALL' 'NoNewPrivileges=true' 'PublishPort=127.0.0.1:8787:8787'; do
    grep -Fxq "$line" "$f" || return 1
  done
}
assert_true hub_quadlet_uses_the_ghcr_image_and_keeps_its_hardening quadlet_uses_the_ghcr_image_and_keeps_its_hardening

# --- third-party-notices.mjs (pure checks through --fixture; the full run needs cargo-about) ---
notices="$root/tools/third-party-notices.mjs"

d="$tmp/notices"
mkdir -p "$d"
cat >"$d/ok.json" <<'JSON'
[{"name":"svelte","version":"5.0.0","license":"MIT","licenseText":"MIT License text"},
 {"name":"@xterm/xterm","version":"6.0.0","license":"MIT","licenseText":"Copyright xterm"}]
JSON
cat >"$d/missing.json" <<'JSON'
[{"name":"svelte","version":"5.0.0","license":"MIT","licenseText":"MIT License text"},
 {"name":"@xterm/xterm","version":"6.0.0","license":"MIT"}]
JSON
cat >"$d/blank.json" <<'JSON'
[{"name":"svelte","version":"5.0.0","license":"MIT","licenseText":"   "}]
JSON
assert_exit notices_accept_a_package_list_with_licence_texts 0 "### svelte 5.0.0 (MIT)" node "$notices" --fixture "$d/ok.json"
assert_exit notices_fail_when_a_bundled_package_has_no_licence_text 1 "no licence text for: @xterm/xterm@6.0.0" node "$notices" --fixture "$d/missing.json"
assert_exit notices_fail_when_a_licence_text_is_blank 1 "no licence text for: svelte@5.0.0" node "$notices" --fixture "$d/blank.json"
printf '## Rust crates\n\n### MIT License\n' >"$d/rust-ok.md"
printf '## Rust crates\n\nnothing\n' >"$d/rust-empty.md"
assert_exit notices_accept_cargo_about_output_with_licence_sections 0 "" node "$notices" --check-rust "$d/rust-ok.md"
assert_exit notices_fail_when_cargo_about_output_has_no_licence_section 1 "cargo about produced no licence sections" node "$notices" --check-rust "$d/rust-empty.md"
assert_exit notices_fail_without_an_output_argument 1 "usage" node "$notices"

# smoke-installed.*: only the safety stops are tested here; the installs run on the hosted runner.
assert_exit smoke_sh_refuses_outside_github_actions 1 "refusing to run outside GitHub Actions" env -u GITHUB_ACTIONS bash "$root/tools/smoke-installed.sh" "$tmp"
assert_exit smoke_sh_refuses_when_github_actions_is_not_true 1 "refusing to run outside GitHub Actions" env GITHUB_ACTIONS=false bash "$root/tools/smoke-installed.sh" "$tmp"
assert_exit smoke_sh_needs_an_installers_dir_in_ci 2 "usage" env GITHUB_ACTIONS=true bash "$root/tools/smoke-installed.sh"
ps=""
for candidate in pwsh powershell; do
  if command -v "$candidate" >/dev/null 2>&1; then ps="$candidate"; break; fi
done
if [ -n "$ps" ]; then
  assert_exit smoke_ps1_refuses_outside_github_actions 1 "refusing to run outside GitHub Actions" env -u GITHUB_ACTIONS "$ps" -NoProfile -File "$root/tools/smoke-installed.ps1" -Installers "$tmp"
else
  echo "skip smoke_ps1_refuses_outside_github_actions: no PowerShell on PATH"
fi

# check-no-build-paths.sh: synthetic binaries, the script only greps bytes.
nbp="$root/tools/check-no-build-paths.sh"
d="$tmp/nbp"
mkdir -p "$d"
printf '%s\n' 'clean binary /build/src/main.rs /cargo/registry/src/x.rs' >"$d/clean.bin"
printf '%s\n' 'panic at /home/runner/.cargo/registry/x.rs' >"$d/linux.bin"
printf '%s\n' 'panic at C:\Users\runneradmin\.cargo\registry\x.rs' >"$d/windows.bin"
assert_exit no_build_paths_accepts_a_clean_binary 0 "" bash "$nbp" "$d/clean.bin"
assert_exit no_build_paths_rejects_a_runner_home_path 1 "/home/runner" bash "$nbp" "$d/linux.bin"
assert_exit no_build_paths_rejects_a_windows_registry_path 1 "runneradmin" bash "$nbp" "$d/windows.bin"
assert_exit no_build_paths_fails_for_a_missing_file 2 "" bash "$nbp" "$d/nope.bin"
# An unreadable file must not count as clean: grep exits 2 and prints nothing, which used to read as zero matches.
printf '%s\n' 'panic at /home/runner/.cargo/registry/x.rs' >"$d/locked.bin"
chmod 000 "$d/locked.bin" 2>/dev/null
if [ "$(id -u)" -eq 0 ]; then
  echo "skip no_build_paths_fails_when_grep_cannot_read_the_file: running as root, chmod 000 does not stop root"
elif cat "$d/locked.bin" >/dev/null 2>&1; then
  echo "skip no_build_paths_fails_when_grep_cannot_read_the_file: chmod 000 has no effect here (Git Bash on Windows)"
else
  assert_exit no_build_paths_fails_when_grep_cannot_read_the_file 2 "cannot read" bash "$nbp" "$d/locked.bin"
fi
chmod 600 "$d/locked.bin" 2>/dev/null

echo
echo "$((total - failures))/$total passed"
[ "$failures" -eq 0 ]

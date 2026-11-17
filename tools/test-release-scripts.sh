#!/usr/bin/env bash
# Tests for tools/check-workflows.sh, check-release-version.sh, verify-installers.sh and third-party-notices.mjs.
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

# --- check-release-version.sh ---
check_version="$root/tools/check-release-version.sh"
cur="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p;}' "$root/Cargo.toml" | head -n 1)"

assert_exit check_release_version_accepts_the_matching_tag 0 "" bash "$check_version" "v$cur"
assert_exit check_release_version_accepts_the_tag_without_v 0 "" bash "$check_version" "$cur"
assert_exit check_release_version_rejects_a_mismatching_tag 1 "does not match" bash "$check_version" "v99.99.99"

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

echo
echo "$((total - failures))/$total passed"
[ "$failures" -eq 0 ]

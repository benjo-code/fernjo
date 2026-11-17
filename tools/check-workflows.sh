#!/usr/bin/env bash
# Policy check for GitHub Actions workflows. Usage: tools/check-workflows.sh [dir]
# Prints `file:line: reason` and exits 1 when a workflow
#   - uses an action that is not pinned to a 40-hex commit SHA (local ./ actions are fine),
#   - mentions self-hosted runners,
#   - triggers on pull_request_target,
#   - has no top-level `permissions:`,
#   - interpolates ${{ github.event.* }} or ${{ inputs.* }} inside a `run:` script
#     (such values must go through `env:`).
set -euo pipefail

dir="${1:-.github/workflows}"
shopt -s nullglob
files=("$dir"/*.yml "$dir"/*.yaml)
if [ "${#files[@]}" -eq 0 ]; then
  echo "check-workflows: no workflow files in $dir" >&2
  exit 1
fi

status=0
for f in "${files[@]}"; do
  out="$(awk -v file="$f" '
    function report(msg) { printf "%s:%d: %s\n", file, NR, msg; bad = 1 }
    BEGIN { in_run = 0; has_perm = 0 }
    {
      line = $0
      if (line ~ /^[ \t]*#/) next                      # full-line comment
      if (line ~ /^permissions:/) has_perm = 1

      # leave a run block when indentation drops back to the key level or less
      match(line, /^[ ]*/); indent = RLENGTH
      if (in_run && line !~ /^[ \t]*$/ && indent <= run_indent) in_run = 0

      if (line ~ /^[ ]*(- )?uses:/) {
        v = line
        sub(/^[ ]*(- )?uses:[ ]*/, "", v)
        gsub(/["\047]/, "", v)
        sub(/[ ]+#.*$/, "", v)
        if (v !~ /^\.\// && v !~ /@[0-9a-f]{40}$/)
          report("action \"" v "\" is not pinned to a 40-character commit SHA")
      }
      if (line ~ /self-hosted/) report("self-hosted runners are not allowed")
      if (line ~ /pull_request_target/) report("pull_request_target is not allowed")

      if (line ~ /^[ ]*(- )?run:/) {
        run_indent = indent
        if (line ~ /^[ ]*- /) run_indent = indent + 2
        rest = line
        sub(/^[ ]*(- )?run:[ ]*/, "", rest)
        if (rest ~ /^[|>][-+0-9]*[ ]*$/) in_run = 1
        else if (rest ~ /\$\{\{[ ]*(github\.event\.|inputs\.)/)
          report("${{ github.event.* }} / ${{ inputs.* }} inside run: pass it through env: instead")
        next
      }
      if (in_run && line ~ /\$\{\{[ ]*(github\.event\.|inputs\.)/)
        report("${{ github.event.* }} / ${{ inputs.* }} inside run: pass it through env: instead")
    }
    END { if (!has_perm) { printf "%s:1: no top-level permissions: block\n", file; bad = 1 } exit bad }
  ' "$f")" || status=1
  [ -n "$out" ] && printf '%s\n' "$out"
done
exit "$status"

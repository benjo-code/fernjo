#!/usr/bin/env bash
# Policy check for GitHub Actions workflows. Usage: tools/check-workflows.sh [dir]
# Prints `file:line: reason` and exits 1 when a workflow
#   - uses an action that is not pinned to a 40-hex commit SHA (local ./ actions are fine),
#   - mentions self-hosted runners,
#   - triggers on pull_request_target,
#   - has no top-level `permissions:`,
#   - uses a ${{ }} expression inside a `run:` script that mentions, anywhere in it, anything but
#     matrix.*, runner.*, env.*, secrets.*, github.event_name, github.sha, github.repository or
#     github.workspace (so github.event.*, inputs.*, github.head_ref, github.ref_name, github.ref,
#     steps.* / needs.* outputs and github[...] indexing must go through `env:`).
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
    function pinned(v,   h) {
      if (v !~ /@[0-9a-f]+$/) return 0
      h = v; sub(/^.*@/, "", h)
      return length(h) == 40
    }
    function allowed(id) {
      if (id ~ /^(matrix|runner|env|secrets)\.[A-Za-z0-9_.-]+$/) return 1
      if (id == "github.event_name" || id == "github.sha" || id == "github.repository" || id == "github.workspace") return 1
      if (id == "true" || id == "false" || id == "null") return 1
      return 0
    }
    # Flags every expression in text that mentions anything outside the allowlist, wherever
    # in the expression it appears (also inside functions, and github[...] indexing). The
    # closing }} is found by walking the text with quote state, so a } inside a string literal
    # (format(\047{0}\047, ...)) cannot hide an expression; an expression that never closes
    # on the line is itself a violation (fail closed). Literals are blanked before the
    # identifier scan; only format/contains/startsWith/endsWith may be called.
    function check_exprs(text,   p, i, n, c, q, e, s, id, rest, nx, done, why) {
      while ((p = index(text, "${{")) > 0) {
        text = substr(text, p + 3)
        n = length(text); q = 0; e = ""; s = ""; done = 0
        for (i = 1; i <= n; i++) {
          c = substr(text, i, 1)
          if (c == "\047") { q = !q; e = e c; continue }
          if (q) { e = e c; continue }
          if (c == "}" && substr(text, i + 1, 1) == "}") { done = 1; break }
          e = e c; s = s c
        }
        gsub(/^[ ]+|[ ]+$/, "", e)
        if (!done) {
          report("expression \"${{ " e "\" is not closed on this line, cannot be checked; keep expressions in run: on one line or use env:")
          return
        }
        text = substr(text, i + 2)
        why = ""
        while (match(s, /[A-Za-z_][A-Za-z0-9_.-]*/)) {
          id = substr(s, RSTART, RLENGTH)
          rest = substr(s, RSTART + RLENGTH)
          s = rest
          sub(/^[ ]+/, "", rest)
          nx = substr(rest, 1, 1)
          if (nx == "[") why = "indexing"
          else if (nx == "(") { if (id !~ /^(format|contains|startsWith|endsWith)$/) why = "function" }
          else if (!allowed(id)) why = "context"
          if (why != "") break
        }
        if (why != "")
          report("expression \"${{ " e " }}\" is not allowed inside run: pass it through env: instead")
      }
    }
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
        if (v !~ /^\.\// && !pinned(v))
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
        else check_exprs(rest)
        next
      }
      if (in_run) check_exprs(line)
    }
    END { if (!has_perm) { printf "%s:1: no top-level permissions: block\n", file; bad = 1 } exit bad }
  ' "$f")" || status=1
  [ -n "$out" ] && printf '%s\n' "$out"
done
exit "$status"

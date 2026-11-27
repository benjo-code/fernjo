#!/usr/bin/env bash
# Policy check for GitHub Actions workflows. Usage: tools/check-workflows.sh [dir]
# Prints `file:line: reason` and exits 1 when a workflow
#   - uses an action that is not pinned to a 40-hex commit SHA (local ./ actions are fine),
#   - mentions self-hosted runners,
#   - triggers on pull_request_target or workflow_run,
#   - has no top-level `permissions:`,
#   - names a job `environment:` other than exactly `release-signing` (the signing job of the release
#     workflow; an object or flow form, a quoted or spaced key, or any other name, is refused),
#   - uses a ${{ }} expression inside a `run:` script that mentions, anywhere in it, anything but
#     matrix.*, runner.*, env.*, secrets.*, github.event_name, github.sha, github.repository or
#     github.workspace (so github.event.*, inputs.*, github.head_ref, github.ref_name, github.ref,
#     steps.* / needs.* outputs and github[...] indexing must go through `env:`).
#     The same rule covers actions/github-script's `script:`, every line of a run value (comment-looking
#     lines and the continuation lines of a plain or quoted scalar included) and a YAML comment after
#     the block indicator (`run: | # note`).
#   - writes a run:, script: or uses: key in a form this line-based check cannot read: flow style
#     (`- { uses: ... }`, `steps: [ ... ]`), a quoted or spaced key, a quoted value that does not close
#     on its line, YAML explicit keys (`? key`), a quoted key containing a backslash escape, or YAML
#     anchors, aliases and merge keys anywhere outside a run value. This is a line-based check, not a
#     YAML parser: it refuses the forms it knows it cannot read, it does not prove that every other
#     form is read correctly, so workflows are written in plain block style.
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
    # A quoted scalar (first character a single or a double quote) that ends on this line.
    function quote_closed(v,   q, i, n, c) {
      q = substr(v, 1, 1); n = length(v)
      for (i = 2; i <= n; i++) {
        c = substr(v, i, 1)
        if (q == "\"" && c == "\\") { i++; continue }
        if (c == q) {
          if (q == "\047" && substr(v, i + 1, 1) == "\047") { i++; continue }
          return 1
        }
      }
      return 0
    }
    BEGIN { in_run = 0; has_perm = 0 }
    {
      line = $0
      if (line ~ /^permissions:/) has_perm = 1

      # Leave a run value when indentation drops back to the key level or less. Everything more
      # indented than a run: or script: key is its value (block scalar, or the continuation lines
      # of a plain or quoted one), comment-looking lines included: GitHub expands expressions there too.
      match(line, /^[ ]*/); indent = RLENGTH
      if (in_run && line !~ /^[ \t]*$/ && indent <= run_indent) in_run = 0
      if (!in_run && line ~ /^[ \t]*#/) next           # full-line comment outside a run value

      if (line ~ /self-hosted/) report("self-hosted runners are not allowed")
      if (line ~ /pull_request_target/) report("pull_request_target is not allowed")
      if (line ~ /workflow_run/) report("workflow_run is not allowed")

      if (in_run) { check_exprs(line); next }

      # The checks below read block-style keys only. Anything that would hide a step key from them
      # fails closed: flow mappings or sequences, quoted or spaced keys, anchors, aliases, merge keys.
      if (line ~ /^[ ]*(- )?\?([ ]|$)/)
        report("a YAML explicit key (?) can hide a run:, script: or uses: key and cannot be checked; write plain block keys")
      if (line ~ /(^[ ]*(- )?|[{,][ ]*)"[^"]*\\[^"]*"[ \t]*:([ \t]|$)/)
        report("a quoted key containing a backslash escape can spell run, script or uses and cannot be checked; write plain block keys")
      canonical = (line ~ /^[ ]*(- )?(uses|run|script):/)
      if (!canonical && line ~ /(^|[^A-Za-z0-9_-])["\047]?(uses|run|script)["\047]?[ \t]*:([ \t]|$)/) {
        if (line ~ /[{[][^"\047]*(uses|run|script)/)
          report("a run:, script: or uses: key in a flow-style mapping or sequence cannot be checked; use block style")
        else
          report("a quoted or unusual run:, script: or uses: key cannot be checked; write it as a plain block key")
      }
      if (!canonical && (line ~ /(^|[ [{,])[&*][A-Za-z0-9_]/ || line ~ /<<[ ]*:/))
        report("YAML anchors, aliases and merge keys are not allowed (an alias hides what it expands to)")

      # a key spelled so that the exact check below does not see it fails closed as well
      if (line !~ /^[ ]*environment:/ && line ~ /(^|[^A-Za-z0-9_-])["\047]?environment["\047]?[ \t]*:([ \t]|$)/)
        report("a quoted or spaced environment key cannot be checked; write the plain block key environment: release-signing")

      if (line ~ /^[ ]*environment:/) {
        v = line
        sub(/^[ ]*environment:[ ]*/, "", v)
        sub(/[ ]+#.*$/, "", v)
        if (v != "release-signing")
          report("environment \"" v "\" is not allowed: only the plain form environment: release-signing is accepted")
      }

      if (line ~ /^[ ]*(- )?uses:/) {
        v = line
        sub(/^[ ]*(- )?uses:[ ]*/, "", v)
        gsub(/["\047]/, "", v)
        sub(/[ ]+#.*$/, "", v)
        if (v !~ /^\.\// && !pinned(v))
          report("action \"" v "\" is not pinned to a 40-character commit SHA")
      }

      # run: and actions/github-script\047s script: are both code with the same rules.
      if (line ~ /^[ ]*(- )?(run|script):/) {
        run_indent = indent
        if (line ~ /^[ ]*- /) run_indent = indent + 2
        rest = line
        sub(/^[ ]*(- )?(run|script):[ ]*/, "", rest)
        in_run = 1
        if (rest ~ /^[|>][-+0-9]*[ ]*(#.*)?$/) next   # block scalar, the comment is YAML
        if (rest ~ /^[&*!]/)
          report("a run: or script: value that is an anchor, alias or tag cannot be checked")
        if (rest ~ /^["\047]/ && !quote_closed(rest))
          report("a quoted run: or script: value that is not closed on this line cannot be checked; use a block scalar (run: |)")
        check_exprs(rest)
        next
      }
    }
    END { if (!has_perm) { printf "%s:1: no top-level permissions: block\n", file; bad = 1 } exit bad }
  ' "$f")" || status=1
  [ -n "$out" ] && printf '%s\n' "$out"
done
exit "$status"

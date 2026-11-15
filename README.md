# <Project name>

<Short description: what does the project do, and for whom?>

**Status:** in development – see [Docs/ROADMAP.md](Docs/ROADMAP.md)

## Setup

<Prerequisites, build, run>

## License

<License or "Proprietary – all rights reserved">

---

## New project from this template

> Delete this section after setup.

> [!IMPORTANT]
> Always create repos in the **`benjo-code`** organization. Without the owner prefix (`gh`) or without selecting "Owner: benjo-code" (browser), the repo ends up under the personal account.

Run all commands in Git Bash and replace `<name>` with the repo name.

### 1. Create and clone the repo

```bash
gh repo create benjo-code/<name> --template benjo-code/project-template --private --clone
cd <name>
git pull origin main   # if the clone is empty: wait a moment, then run again
gh repo view benjo-code/<name> --json owner -q .owner.login   # must print "benjo-code"
```

### 2. Create `dev`, set it as default branch, configure merge options

```bash
git switch -c dev
git push -u origin dev
gh repo edit benjo-code/<name> --default-branch dev --enable-merge-commit --enable-squash-merge --enable-rebase-merge=false --delete-branch-on-merge
```

### 3. Branch protection for `main` and `dev`

PR required, no force push, no deletion. Admins are exempt (`enforce_admins: false`); the rule "never directly on `main`/`dev`" from `CLAUDE.md` still applies. The `roadmap` check (workflow `.github/workflows/roadmap.yml`, shipped with the template) is required from the start; the CI check for build and test is added in M0 once the CI exists.

```bash
REPO=benjo-code/<name>
for b in main dev; do
gh api -X PUT "repos/$REPO/branches/$b/protection" --input - <<'EOF'
{
  "required_status_checks": { "strict": true, "contexts": ["roadmap"] },
  "enforce_admins": false,
  "required_pull_request_reviews": { "required_approving_review_count": 0 },
  "restrictions": null,
  "allow_force_pushes": false,
  "allow_deletions": false
}
EOF
done
```

From now on nothing goes directly to `main` or `dev`; everything goes through feature branches and PRs.

### 4. Get started

Tell Claude Code **"Plan milestone 0"**. Claude creates the branch `feature/m00-…` for it. That is also where the header of `CLAUDE.md`, this README, and the placeholders in `Docs/roadmap.json` (then `dotnet run tools/roadmap.cs -- generate`) and `Docs/DECISIONS.md` get filled in.

### 5. Roadmap tool

The roadmap lives in `Docs/roadmap.json`; `Docs/ROADMAP.md` is generated with `dotnet run tools/roadmap.cs -- generate` (see "Maintaining the roadmap" in `CLAUDE.md`). This requires the .NET 10 SDK on the development machine, even if the project otherwise uses a different stack. The `roadmap` job runs on the organization's runners with the label `dotnet-linux`. A project does not need the `tools/roadmap-fixtures/` folder; it only tests the template.

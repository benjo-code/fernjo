<!-- Generated from Docs/roadmap.json with `dotnet run tools/roadmap.cs -- generate` – do not edit by hand. -->
# <Project name> – Roadmap

Overall roadmap for the project. Every milestone is sized so that it can be completed in **one** plan and one round of implementation.

> **This file is generated.** The source is [roadmap.json](roadmap.json) (schema: [roadmap.schema.json](roadmap.schema.json)). Edit only that file, then run `dotnet run tools/roadmap.cs -- generate`.

## How we work

1. **“Plan milestone N”:** Claude Code reads this roadmap, the linked spec and the current state of the code, and writes a plan to `Docs/superpowers/plans/YYYY-MM-DD-milestone-NN-<topic>.md`. The plan is linked from the milestone, status → `🟡 planned`.
2. **Implementation:** Work through the plan. Tick off every finished task immediately (`done: true` in `Docs/roadmap.json`).
3. **Completion:** Once all acceptance criteria are verified and ticked off → status `✅ done`, enter the date.
4. Scope changes during implementation are recorded under “Scope changes”, not silently in the code.

**Status legend:** `⬜ open` · `🟡 planned` · `🔵 in progress` · `✅ done`

**Rules:**
- Work through milestones in order as a rule. The prerequisites listed on each milestone are what counts.
- A milestone starts with `Create spec` if its subproject has no spec yet. The spec is developed through brainstorming and stored under `Docs/superpowers/specs/`.
- Tasks of later subprojects are deliberately kept coarse and are refined when the respective spec is written.

## Overview

| # | Milestone | Subproject | Status | Plan | Done on |
|---|---|---|---|---|---|
| 0 | Repo scaffold & CI | 1 – <Name> | ⬜ open | – | – |

## Scope changes

_None yet._

---

# Subproject 1 – <Name>

**Spec:** not created yet (created in M0)

## Milestone 0 – Repo scaffold & CI
**Status:** ⬜ open · **Plan:** – · **Prerequisites:** none

Goal: An empty but fully set-up repo that all further milestones build on.

**Tasks**
- [ ] Create spec (brainstorming): goal, scope, stack, subprojects; link it here and enter the first milestones
- [ ] Repo setup per README done: `dev` as default branch, merge options, branch protection for `main` and `dev`
- [ ] Fill in the project header in `CLAUDE.md` and `README.md`, remove the template section from the README
- [ ] Stack-specific `.gitignore` (incl. `.env`) and `.gitattributes` additions (e.g. Unity: Git LFS for large assets)
- [ ] Project scaffold incl. test project
- [ ] `.env.example` with placeholders, if secrets are needed
- [ ] CI (GitHub Actions): build, test and format/lint on PRs to `dev` and `main`; the `roadmap` job from `.github/workflows/roadmap.yml` stays
- [ ] Add the CI jobs (`roadmap` and the job for build, test, lint) as required status checks in the branch protection of `main` and `dev`
- [ ] Dependabot for the package ecosystem and GitHub Actions

**Acceptance criteria**
- [ ] Build, test and format/lint run green locally (commands in `CLAUDE.md`)
- [ ] CI is green on a PR to `dev`
- [ ] Branch protection for `main` and `dev` is active: PR required, CI check required, no force push, no deletion – `gh api repos/benjo-code/<name>/branches/<branch>/protection`

## Backlog

Deliberately not scheduled. Ideas that `/ideen` files here are listed; if needed, one becomes a milestone.

_Nothing yet._

<!-- Generated from Docs/roadmap.json with `dotnet run tools/roadmap.cs -- generate` – do not edit by hand. -->
# Complete – Roadmap

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
| 0 | Scaffold & CI | 1 – Foundation \| Core | ✅ done | [Plan](superpowers/plans/2026-10-02-milestone-00-scaffold.md) | 2026-10-02 |
| 5.1 | Reading without sign-in | 1 – Foundation \| Core | 🔵 in progress | [Plan](superpowers/plans/2026-10-02-milestone-05-1-reading.md) | – |
| – | **Release `v0.1.0`** – replaces the old one (M0 to M5.1) | – | ✅ done | – | 2026-10-02 |
| 6 | Agent | 2 – Hosts | 🟡 planned | [Plan](superpowers/plans/2026-10-03-milestone-06-agent.md) | – |
| – | **Release `v0.2.0`** – brings hosts (M6) | – | ⬜ open | – | – |

## Scope changes

- **2026-10-02 (Planning M1):** The PWA arrives in M1.
  Second line of the entry.

---

# Subproject 1 – Foundation | Core

**Spec:** [2026-10-02-design.md](superpowers/specs/2026-10-02-design.md) (sections 1 to 6)

## Milestone 0 – Scaffold & CI
**Status:** ✅ done (2026-10-02) · **Plan:** [2026-10-02-milestone-00-scaffold.md](superpowers/plans/2026-10-02-milestone-00-scaffold.md) · **Prerequisites:** none

Goal: A set-up repo.

**Tasks**
- [x] Create project scaffold
- [x] Set up CI
  with a second line

**Acceptance criteria**
- [x] Build is green – `dotnet build` exit 0
- [x] No evidence needed

## Milestone 5.1 – Reading without sign-in
**Status:** 🔵 in progress · **Plan:** [2026-10-02-milestone-05-1-reading.md](superpowers/plans/2026-10-02-milestone-05-1-reading.md) · **Spec:** [2026-10-02-reading-design.md](superpowers/specs/2026-10-02-reading-design.md) · **Prerequisites:** M0 (Release `v0.1.0`)

Goal: Reading works without signing in.

**Tasks**
- [ ] Create policy

**Acceptance criteria**
- [ ] Tests are green

**Open questions**
Still to be clarified: page protection.

## Release `v0.1.0` – Acceptance

Manual check by the user.

- [ ] Runs in production
- [x] Data migrated – Report of 2026-10-03

**Known limits**
Only one host tested.

---

# Subproject 2 – Hosts

## Milestone 6 – Agent
**Status:** 🟡 planned · **Plan:** [2026-10-03-milestone-06-agent.md](superpowers/plans/2026-10-03-milestone-06-agent.md) · **Prerequisites:** M0, M5.1

Goal: An agent sends metrics.

**Tasks**
- [ ] Build agent

## Backlog

Deliberately not scheduled. Ideas that `/ideen` files here are listed; if needed, one becomes a milestone.

- **Dark theme:** Follows the system setting. (Idea #12)
- **Export:** As CSV.

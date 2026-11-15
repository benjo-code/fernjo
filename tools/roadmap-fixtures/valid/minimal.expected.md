<!-- Generated from Docs/roadmap.json with `dotnet run tools/roadmap.cs -- generate` – do not edit by hand. -->
# Example – Roadmap

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
| 0 | Repo scaffold | 1 – Foundation | ⬜ open | – | – |

## Scope changes

_None yet._

---

# Subproject 1 – Foundation

## Milestone 0 – Repo scaffold
**Status:** ⬜ open · **Plan:** – · **Prerequisites:** none

Goal: An empty, set-up repo.

## Backlog

Deliberately not scheduled. Ideas that `/ideen` files here are listed; if needed, one becomes a milestone.

_Nothing yet._

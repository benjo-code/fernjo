# Changelog

All notable changes to this project are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versioning according to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Every release entry names the milestones it contains.

## [Unreleased]

### Added
- Section `## Backlog` in `Docs/ROADMAP.md`, step "review open ideas with `/ideen`" in the milestone workflow of `CLAUDE.md`, and issue template `idee.md` (label `idee`)
- Roadmap as machine-readable `Docs/roadmap.json` with a JSON schema (`Docs/roadmap.schema.json`); `tools/roadmap.cs` (.NET 10 single file) generates `Docs/ROADMAP.md` from it (commands `generate`, `check`, `selftest`), the `roadmap` job in `.github/workflows/roadmap.yml` validates it, section "Maintaining the roadmap" in `CLAUDE.md`

### Changed
- `Docs/ROADMAP.md` is generated and is no longer edited by hand; Labjo (M12) migrates existing repos
- Everything in the template is now in English: documentation, templates, `CLAUDE.md`, the generated roadmap, and the messages of `tools/roadmap.cs`

### Fixed

### Removed

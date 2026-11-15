# Changelog

All notable changes to this project are documented here.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versioning according to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Every release entry names the milestones it contains.

## [Unreleased]

### Added

- Visual design "Ulm, revised": light and dark theme that follows the system setting, bundled fonts (Hanken Grotesk, Red Hat Mono), design tokens for all upcoming UI (M1).
- New app icon (M1).
- Connection tree (M2): folders and SSH/RDP connections in a sidebar tree with filter, expand/collapse, keyboard navigation, rename, duplicate, delete with confirmation, context menu and drag and drop; the tree is saved to `connections.json` in the app's data folder (atomic write with a `.bak` backup).
- Inherited settings (M2): username, port and keepalive can be set on a folder and are inherited by everything below it; the properties panel shows "from <folder>" and an "Inherit" reset.
- Single instance (M2): starting fernjo a second time brings the running window to the front.

### Changed

### Fixed

### Removed

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Read solids with internal voids (`BREP_WITH_VOIDS`): a void solid now appears
  in the tree with its outer shell, and each internal cavity is shown as a
  `Void` group of its faces (requires step-io 0.2.3).

### Changed

- Bump `step-io` to 0.2.3.

## [0.1.0] - 2026-07-07

### Added

- Single-file, in-browser STEP (ISO 10303) viewer built on
  [step-io](https://github.com/elgar328/step-io): b-rep geometry, the assembly tree,
  PMI (features, dimensions, tolerances, datums), and a provenance report,
  rendered with three.js.
- Load a model via the `?file=<url>` query parameter, the **Open STEP file**
  button, or drag-and-drop; responsive layout for narrow (mobile) screens.

[Unreleased]: https://github.com/elgar328/step-loupe/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/elgar328/step-loupe/releases/tag/v0.1.0

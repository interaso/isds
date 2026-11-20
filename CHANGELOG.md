# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Optional `tracing` instrumentation with privacy-safe request spans and response metadata.

## [0.1.0] - 2026-11-20

### Added

- Asynchronous, cheaply cloneable client with explicit test or production environment selection.
- Username/password authentication over TLS using `reqwest` and rustls, without native dependencies.
- Databox state checks, exact and criteria-based lookup, owner information, current credit, and credit history.
- Standard and large-message sending with automatic or forced mode selection, decoded-attachment limits, and verified large-attachment upload hashes.
- Sent and received message listing, automatic or forced message download, envelope download, delivery information, and mark-read operations.
- Validated message and databox identifiers, descriptive databox types, and optional serde support for identifiers.
- Structured transport, HTTP, SOAP, service-status, size-limit, and response-decoding errors.
- Consumer examples, offline tests, and opt-in lifecycle tests for the official ISDS test environment.

[Unreleased]: https://github.com/interaso/isds/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/interaso/isds/releases/tag/v0.1.0

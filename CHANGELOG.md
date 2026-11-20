# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Asynchronous, cheaply cloneable client with explicit test or production environment selection.
- Username/password authentication over TLS using `reqwest` and rustls, without `libdatovka`; the current backend builds bundled native code through `aws-lc-rs`/`aws-lc-sys` and may require a supported C/CMake toolchain.
- Databox state checks, exact and criteria-based lookup, owner information, current credit, and credit history.
- Standard and large-message sending with automatic or forced mode selection, decoded-attachment limits, and verified large-attachment upload hashes.
- Sent and received message listing, automatic or forced message download, envelope download, delivery information, and mark-read operations.
- Validated message and databox identifiers, descriptive databox types, and optional serde support for identifiers.
- Structured transport, HTTP, SOAP, service-status, size-limit, and response-decoding errors.
- Consumer examples, offline tests, and opt-in lifecycle tests for the official ISDS test environment.
- Optional `tracing` instrumentation with privacy-safe request spans and response metadata.
- Typed service-error classification for messages not yet downloadable and delivery still in progress.
- Continuous integration for individual optional features and all-feature `x86_64-unknown-linux-musl` builds.

### Changed

- Standardized public date/time compatibility on `jiff` 0.2 (currently 0.2.46), allowing downstream `jiff` 0.2 values to be used directly.

### Fixed

- Disabled reqwest protocol-NACK retries, preserving the no-automatic-retry guarantee under downstream HTTP/2 or HTTP/3 feature unification.
- Hardened responses with requested-message ID correlation, complete scalar text, standalone inline-XML namespaces, singleton cardinality, attachment-count checks, bounded XML complexity, and token-safe parser errors.
- Made authentication errors take precedence over body-read failures and redacted both credential fields from debugging output.
- Corrected the credit-history example to query today and the previous six `Europe/Prague` calendar dates, with tomorrow as the exclusive upper bound.
- Switched inbox retry handling and live-test polling to typed service-error kinds.
- Documented returned-message ID validation and corrected native-build and dependency-logging guidance.

[Unreleased]: https://github.com/interaso/isds

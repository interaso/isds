# Contributing

Thank you for contributing to `isds`. Please keep changes focused, document public APIs, and add or update tests for behavior changes.

## Development setup

Install a current stable Rust toolchain with `rustfmt` and Clippy. The crate uses the Rust 2024 edition, supports Rust 1.89 and newer, and forbids unsafe code. Check the declared MSRV after dependency updates.

Run the same offline checks as CI before submitting a change:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --all-features --locked
cargo check --examples --locked
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps --locked
cargo package --locked --allow-dirty
```

The default and all-feature test suites do not contact ISDS. Keep unit and integration tests independent of local credentials and external services. CI also checks each optional feature independently, checks all targets and features at Rust 1.89, and checks the all-feature crate for `x86_64-unknown-linux-musl` with `musl-gcc`; equivalent local musl checks require the Rust target and a musl C toolchain.

## Public API and protocol changes

- Prefer descriptive English API names and established ecosystem crates over protocol reimplementation.
- Preserve explicit environment selection and never make production the default.
- Do not add automatic retries to operations that create or modify messages. Their outcomes can be ambiguous after transport failures.
- Keep credentials and attachment contents out of logs and debugging output.
- Update README examples, crate documentation, and `CHANGELOG.md` when changing consumer-visible behavior.
- Record official schema source URLs and hashes in `SCHEMAS.md` when updating the protocol basis.

## Live tests

Live tests are optional maintainer checks, never part of CI. They may use only the official ISDS test environment and require explicit opt-in with `ISDS_RUN_LIVE_TESTS=1`. Supply credentials through your shell or secret manager; never commit credentials or `.env` files.

The lifecycle sends real test messages and can trigger delivery behavior in the test environment. See [README.md](README.md#live-tests-against-the-official-test-environment) for the required variables and serial test command. Do not point live tests at production accounts.

## Changelog and releases

Record consumer-visible changes under `Unreleased` in [CHANGELOG.md](CHANGELOG.md), using categories such as Added, Changed, Fixed, and Removed as appropriate. Do not assign a version or date until the release is being prepared.

Releases are maintainer-only and use the [published-release workflow](.github/workflows/release.yml):

1. Start from a reviewed `main` branch with passing CI. When protocol behavior or wire dependencies changed, run the official test-environment lifecycle manually.
2. Update the version in `Cargo.toml` and the dependency examples in `README.md`.
3. Move the relevant `Unreleased` changelog entries into a `## [X.Y.Z] - YYYY-MM-DD` section, retain an empty `Unreleased` section for future changes, and update the changelog comparison and release links.
4. Commit and review these changes. Prepare a GitHub release from the release commit with a new or existing `vX.Y.Z` tag exactly matching the package version, and use the dated changelog section for its notes. Drafts can be reviewed before publication. crates.io releases are immutable, so publish the GitHub release only when ready.
5. Publishing the GitHub release triggers the workflow. It checks out the release tag, validates that the tag matches the package version and an exact dated changelog heading exists, runs the full offline verification matrix and publication dry-run, and publishes only to crates.io. It never creates or modifies the GitHub release and never runs live tests.
6. Verify the published crates.io page, docs.rs build, GitHub release notes, and rendered documentation links. If crate publication fails, the existing GitHub release remains in place while the workflow is fixed or rerun.

The GitHub repository must have a suitably scoped `CARGO_REGISTRY_TOKEN` Actions secret with permission to publish the crate. Keep release credentials in the repository's secret store, never in workflow files or other committed content.

## Licensing

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this crate is licensed under the MIT License, without additional terms or conditions.

# Contributing

Thank you for contributing to `isds`. Please keep changes focused, document public APIs, and add or update tests for behavior changes.

## Development setup

Install a current stable Rust toolchain with `rustfmt` and Clippy. The crate uses the Rust 2024 edition and forbids unsafe code.

Run the same offline checks as CI before submitting a change:

```text
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --all-features --locked
cargo check --examples --locked
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps --locked
cargo package --locked --allow-dirty
```

The default and all-feature test suites do not contact ISDS. Keep unit and integration tests independent of local credentials and external services.

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

## Licensing

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this crate is licensed under the MIT License, without additional terms or conditions.

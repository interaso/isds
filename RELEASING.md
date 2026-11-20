# Release process

Releases are manual. The repository intentionally has no workflow that publishes to crates.io or creates GitHub releases.

## Before every release

1. Start from a clean, reviewed `main` branch with passing CI.
2. Update `Cargo.toml`, the README dependency example, and `CHANGELOG.md` to the intended version. For pre-1.0 releases, increment the minor version for breaking API changes and the patch version for compatible fixes.
3. Review `cargo package --list --allow-dirty` and the complete diff. Confirm that no credentials, local environment files, or unrelated artifacts are included.
4. Run the full offline verification matrix:

   ```text
   cargo fmt --all -- --check
   cargo clippy --locked --all-targets --all-features -- -D warnings
   cargo test --locked
   cargo test --all-features --locked
   cargo check --examples --locked
   RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps --locked
   cargo package --locked --allow-dirty
   cargo publish --dry-run --locked --allow-dirty
   ```

5. When protocol behavior or wire dependencies changed, run the official test-environment lifecycle as described in [README.md](README.md#live-tests-against-the-official-test-environment). Use only test accounts and environment-provided credentials.

## Publish

1. Commit the reviewed release changes and create an annotated `vX.Y.Z` tag.
2. Push the branch and tag, then confirm that CI passes on the release commit.
3. Publish with `cargo publish --locked` using a suitably scoped crates.io token. crates.io releases are immutable; a published version cannot be replaced.
4. Create the GitHub release from the same tag and copy the corresponding changelog section into its notes.
5. Verify the crates.io page and docs.rs build, and check the rendered README and documentation links.

For the initial publication, also verify that the crate name is still available and that the publishing account has permission to publish under the package metadata in `Cargo.toml`.

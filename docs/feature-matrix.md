# Optional feature-combination matrix

`cargo-all-features` is a third-party Cargo subcommand that checks or tests
combinations of a crate's features. It is **optional** and is not part of the
required quality gates, because the generated feature matrix produces a large
`target/` directory and can take a long time on a full workspace.

## Install

```sh
cargo install cargo-all-features
```

The tool is independent of this crate; it is not a runtime or test dependency
and is not vendored.

## Usage

From the repository root:

```sh
# Check every configured feature combination.
cargo all-features check

# Test every configured feature combination.
cargo all-features test
```

Use a separate target directory if you want to keep the main build cache
untouched:

```sh
CARGO_TARGET_DIR=target/feature-matrix cargo all-features check
```

For a large matrix, split the work into feature-set chunks. `--chunk` is
1-based, and `--chunk-granularity feature` splits by `(crate, feature-set)`
rather than by package:

```sh
cargo all-features check --chunk-granularity feature --n-chunks 8 --chunk 1
```

Repeat for chunks 1 through 8. Each chunk can be run in a separate job.

## Configuration

The matrix is bounded in `Cargo.toml` under
`[package.metadata.cargo-all-features]`:

- `max_combination_size = 2` tests every feature alone and every pair, rather
  than the full 2^14 power set.
- `denylist = ["default"]` avoids duplicating the all-feature build, which is
  covered by `cargo test --all-features`.
- `skip_feature_sets` removes redundant `cpp` umbrella pairs, because `cpp`
  already enables `cpp-itanium`, `cpp-msvc`, and `cpp-gnu-v2`.

This is deliberately a smoke/compatibility matrix, not exhaustive proof of all
feature combinations. `cargo test --all-features`, `--no-default-features`, and
the individual feature builds remain the required gates.

## Cleanup

The matrix creates multiple target artifacts. Afterward:

```sh
cargo clean
```

or remove only the dedicated target directory:

```sh
rm -rf target/feature-matrix
```

See the global `manage-rust-disk-resources` skill for safe cleanup guidance.

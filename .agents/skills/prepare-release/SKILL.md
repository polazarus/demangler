---
name: prepare-release
description: Prepare, verify, and publish a new version of the demangle library to crates.io. Use when cutting a release, bumping the version, checking package contents, publishing, tagging, or writing release notes.
---

# Prepare a crates.io release

Use this skill for a library release of `demangle` to crates.io. `AGENTS.md` is
authoritative; this skill is the executable release checklist.

Hard rules:

- A crates.io version is **permanent**. It cannot be replaced or deleted.
- Do not publish, push, tag, or open a release without explicit operator
  confirmation.
- Do not put a token in the repository, shell history, or chat.
- Do not use `--allow-dirty` or `--no-verify` for a real release.
- Stop on the first failed gate. Do not paper over a failure by editing the
  version, lowering a corpus floor, or excluding a failing test.

## 0. Preflight

```sh
cd /home/debian/demangle
git fetch origin --tags
git status --short
git rev-parse --abbrev-ref HEAD       # must be main
git rev-parse HEAD
git rev-parse origin/main             # must match HEAD
```

The working tree must be clean. `.plans/` is ignored, but do not publish from a
dirty or detached checkout.

### Crate name and ownership

The package name is in `Cargo.toml`. Verify it is available or owned before
spending time on a release:

```sh
cargo search <name> --limit 10
cargo info <name>
cargo owner --list <name>             # requires cargo login
```

The current manifest name is `demangle`; a crates.io search already returns an
existing `demangle` crate. Confirm that this account owns it. If it does not,
**stop and rename** the package, library imports, docs and examples before
versioning. Crate names are first-come, first-served and cannot be taken over.

Also check that the version is not already published:

```sh
cargo info <name>                     # latest published version
# or inspect https://crates.io/crates/<name>/versions
```

### Metadata

`Cargo.toml` must have:

- `name`, `version`, `edition`;
- a plain-text `description`;
- `license` (SPDX expression) or `license-file`;
- `repository` and `readme`;
- recommended: `rust-version`, `keywords`, `categories`, `documentation`.

Keep `keywords` to at most five, each ASCII and at most 20 characters. Keep
`categories` to at most five exact crates.io category slugs.

Add an explicit docs.rs configuration so the published documentation builds
with every backend:

```toml
[package.metadata.docs.rs]
all-features = true
```

Verify the runtime dependency graph is empty:

```sh
cargo tree --edges normal
```

The only normal package should be `demangle`; oracle demanglers must stay in
`[dev-dependencies]`.

### Package contents

Run these before versioning and again after the release commit:

```sh
cargo package --list
cargo package --list | wc -l
```

Inspect the list. A library package should contain source, tests, docs, README,
changelog and license material. It must not contain:

- `refs/` or any vendored corpus;
- `.plans/`, `.agents/`, or internal agent instructions;
- `target/`, editor files, secrets, tokens or credentials;
- LFS pointer files outside `refs/`.

`refs` is already excluded in the manifest. If internal files appear, update
`[package].exclude` before proceeding and rerun `cargo package --list`.

`cargo package --list` reports the list but not the size. Build the package and
check the 10 MiB crates.io limit:

```sh
cargo package
ls -lh target/package/<name>-<version>.crate
```

If the package is unexpectedly large, find the included path before removing
it. Do not exclude a test or source file merely to make the size check pass.

## 1. Choose the version

Follow SemVer:

- `0.x` releases may break APIs in a minor bump, but a patch bump must remain
  compatible.
- `1.x+` breaking changes require a major bump.
- A new backend is normally a minor release.
- A bug fix with no API change is a patch release.
- A first release is normally `0.1.0`.

Check the built-in public API docs and `CHANGELOG.md` for breaking changes,
removals, renamed variants, changed defaults and feature-gate changes.

If this is not the first release, run the SemVer checker against the last
published version if the tool is available:

```sh
cargo install cargo-semver-checks
cargo semver-checks check-release --baseline-version <last-version>
```

## 2. Update version and changelog

1. Set `version = "X.Y.Z"` in `Cargo.toml`.
2. Run `cargo check --all-features` so `Cargo.lock` records the package version.
3. Update `CHANGELOG.md`:
   - rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`;
   - add a fresh empty `## [Unreleased]` at the top;
   - keep entries short, imperative and user-facing;
   - call out breaking changes explicitly;
   - update the compare/release links.
4. Update any version examples in README or docs that name the current release.
5. Commit the release preparation:

```sh
git add Cargo.toml Cargo.lock CHANGELOG.md README.md docs
git commit -m "chore(release): prepare X.Y.Z"
```

Do not amend, rebase, force-push, or move an existing tag.

## 3. Full verification

Run the complete repository gate from a clean release commit:

```sh
cargo test --all-features
cargo test --no-default-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace --all-targets --no-default-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --no-default-features
cargo build --lib --no-default-features
cargo build --lib --no-default-features --target thumbv7em-none-eabi
cargo build --release
```

Run every language/dialect feature alone:

```sh
for feature in rust swift go objc kotlin-native d jni ada fortran delphi \
               ocaml coff encoding cpp-itanium cpp-msvc cpp-gnu-v2; do
  cargo test --no-default-features --features "$feature"
done
```

Optional but useful:

```sh
cargo all-features check       # if cargo-all-features is installed
cargo audit                    # if cargo-audit is installed
```

Corpus tests must report the pinned exact counts with **zero wrong** rows. They
may skip only because `refs/` is absent from a packaged archive, never because a
file was accidentally omitted from the repository.

## 4. Verify the packaged crate

Build the upload artifact and inspect it:

```sh
cargo package
cargo package --list
tar -tzf target/package/<name>-<version>.crate | head -50
ls -lh target/package/<name>-<version>.crate
```

Confirm the archive contains `Cargo.toml`, source, tests, docs, README,
CHANGELOG and license material, and does not contain `refs/`, `.agents/`,
`.plans/`, `target/`, tokens or internal notes.

Test the packaged source without `refs/`:

```sh
tmp=$(mktemp -d)
tar -xzf target/package/<name>-<version>.crate -C "$tmp"
cd "$tmp/<name>-<version>"
cargo test --all-features
```

The corpus tests should skip because `refs/` is not packaged; the rest of the
suite must pass. This catches missing files, bad `include`/`exclude` rules and
tests that assume the repository layout.

Then run the real upload check:

```sh
cargo publish --dry-run
```

`--dry-run` performs the package checks and compile verification but does not
upload. Inspect the generated `.crate` and the dry-run output for warnings.

Do not use `--allow-dirty`; commit any fix and rerun the whole package check.

## 5. Human approval gate

Before publishing, report to the operator:

- package name and version;
- release commit SHA and clean-tree status;
- crates.io ownership/name check result;
- package size and file-list summary;
- all validation commands and their result;
- confirmation that `cargo publish --dry-run` passed.

Ask for explicit approval to publish. Publishing is irreversible, so this is a
hard stop. Do not infer approval from “prepare the release”.

## 6. Publish

Authenticate once, without sharing the token:

```sh
cargo login
```

`cargo login` stores the token in `~/.cargo/credentials.toml`. Never commit that
file, paste the token into a command, or put it in a repository file. If a token
leaks, revoke it on crates.io immediately.

Publish from the clean release commit:

```sh
cargo publish
```

If the client times out while waiting for the index, check crates.io before
retrying. The upload may have succeeded. Never publish the same version twice;
crates.io rejects it.

## 7. GitHub Actions and trusted publishing

A release workflow is appropriate, but **do not publish on every push**.

Use two workflows:

1. `ci.yml` on pull requests and `main`: full tests, clippy, rustdoc, `no_std`
   and feature checks, and `cargo package`/`cargo publish --dry-run`.
2. `release.yml` on a protected `v*` tag or a manual dispatch: verify the tag
   matches `Cargo.toml`, run the release gates, publish, then create the GitHub
   release.

The release job must use a protected GitHub environment (`release`) with
required reviewers, and only that job gets `id-token: write` or a crates.io
token.

### First release is different

Trusted Publishing can only be configured **after the crate exists** on
crates.io. For the first release:

- publish manually from the release commit with `cargo login` and `cargo
  publish`; or
- use a narrowly scoped `CARGO_REGISTRY_TOKEN` environment secret for one run.

After the first publish, create a Trusted Publisher on crates.io for:

- owner: `polazarus` (or the final GitHub owner);
- repository: the final repository name;
- workflow: `.github/workflows/release.yml`;
- environment: `release`.

Then remove any long-lived token and let the workflow request a short-lived
token with:

```yaml
permissions:
  contents: write       # create the GitHub release
  id-token: write       # request the crates.io OIDC token

jobs:
  publish:
    environment: release
    steps:
      - uses: actions/checkout@<pinned-sha>
        with:
          lfs: true          # corpus tests need refs/ from Git LFS
          fetch-depth: 0
      - uses: dtolnay/rust-toolchain@<pinned-sha>
      - name: Verify tag matches the package version
        run: test "${GITHUB_REF_NAME#v}" = "$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')"
      - run: cargo test --all-features
      - id: crates-io
        uses: rust-lang/crates-io-auth-action@<pinned-sha>
      - run: cargo publish
        env:
          CARGO_REGISTRY_TOKEN: ${{ steps.crates-io.outputs.token }}
      - run: gh release create "$GITHUB_REF_NAME" --notes-from-tag
        env:
          GH_TOKEN: ${{ github.token }}
```

Pin every action by commit SHA, not a mutable tag. The workflow must check out
LFS data for the full corpus gates; the published `.crate` still excludes
`refs/`.

The GitHub release should be created only after `cargo publish` succeeds. If
the release step fails after a successful publish, rerun only the release
step; never republish the same version.

## 8. Tag and release

After the version appears on crates.io, create an annotated tag on the exact
published commit:

```sh
git tag -a vX.Y.Z -m "demangle X.Y.Z"
git push origin main
git push origin vX.Y.Z
```

If tag signing is configured, use `git tag -s` instead. Tag the commit that was
published; do not move or reuse a tag.

Create the GitHub release from the curator-approved changelog section. Keep the
release notes user-facing: features, fixes, breaking changes and migration
steps. Do not paste internal development history.

## 9. Post-publish verification

```sh
cargo info <name>@X.Y.Z
cargo install <name> --version X.Y.Z --root /tmp/<name>-install
/tmp/<name>-install/bin/demangle "_ZN3Foo3barEv"
```

Check:

- crates.io page renders the README and metadata;
- docs.rs build succeeds for `https://docs.rs/<name>/X.Y.Z`;
- the `demangle` binary installs and runs;
- a temporary consumer project can add the crate and use the public API;
- the tag points at the published commit.

For a temporary consumer:

```sh
tmp=$(mktemp -d)
cd "$tmp"
cargo init --bin consumer
cargo add <name>@X.Y.Z
cargo run
```

## 10. If something is wrong

A yank hides a broken version from new lockfiles but does not delete it:

```sh
cargo yank --version X.Y.Z
cargo yank --version X.Y.Z --undo
```

Yank only after assessing the breakage. Then fix the issue and publish a new
patch version. Never reuse the published version number. Existing lockfiles
continue to resolve a yanked version, so a yank is not a security deletion; if a
secret was published, revoke the secret first.

## References

Official conventions used by this checklist:

- Cargo publishing: <https://doc.rust-lang.org/cargo/reference/publishing.html>
- `cargo publish`: <https://doc.rust-lang.org/cargo/commands/cargo-publish.html>
- Manifest fields (`license`, `readme`, `exclude`, `rust-version`):
  <https://doc.rust-lang.org/cargo/reference/manifest.html>
- docs.rs build metadata:
  <https://docs.rs/about/metadata>
- crates.io policies: <https://crates.io/policies>
- Trusted publishing RFC:
  <https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html>
- crates.io auth action:
  <https://github.com/rust-lang/crates-io-auth-action>
- SemVer: <https://semver.org/>

## 11. Release checklist

- [ ] clean `main`, equal to `origin/main`
- [ ] package name available and owned
- [ ] version not already published
- [ ] required metadata and license present
- [ ] docs.rs `all-features` configured
- [ ] no runtime dependencies
- [ ] `cargo package --list` contains no `refs/`, `.agents/`, `.plans/` or secrets
- [ ] package is below 10 MiB
- [ ] version and changelog updated
- [ ] all-features and no-default tests pass
- [ ] every feature tested alone
- [ ] clippy and rustdoc pass with `-D warnings`
- [ ] no_std and thumb builds pass
- [ ] packaged source tests without `refs/`
- [ ] `cargo publish --dry-run` passes
- [ ] explicit publish approval received
- [ ] `cargo publish` succeeds
- [ ] tag points at the published commit
- [ ] docs.rs and installed binary verified

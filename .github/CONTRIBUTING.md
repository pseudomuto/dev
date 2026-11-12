# Contributing to dev

## Setup

dev is set up with dev. [Install it](../README.md#install) (0.0.2 or later), then from the repo:

```sh
dev up
```

That installs rustup if needed, the Rust toolchain in `rust-toolchain.toml`, nightly rustfmt (`rustfmt.toml` needs
it), and actionlint, cargo-shear, and shellcheck. Their versions are pinned in `dev.yaml`. dev keeps tools in your own
mise config, so these become your default versions everywhere, not only in this repo.

## Commands

`dev --help` lists the project's commands. The main ones:

| Command           | What it does                                     |
| ----------------- | ------------------------------------------------ |
| `dev build`       | Builds a debug binary.                           |
| `dev test`        | Runs the tests. Args go to `cargo test`.         |
| `dev lint`        | Runs clippy, with warnings as errors.            |
| `dev fmt`         | Formats the code.                                |
| `dev test:ci`     | Runs everything CI does: format, lints, tests.   |
| `dev test:docker` | Opens a shell in a throwaway Linux container.    |
| `dev demo`        | Records `demo/demo.gif` in a container.          |

`test:docker` and `demo` need Docker.

Run `dev test:ci` before opening a pull request. CI runs the same thing on Ubuntu and macOS, using dev built from the
pull request itself, so a change to dev and a `dev.yaml` that relies on it can land together.

## Tests

Tests never touch the network or your machine's setup:

- They run `dev` with a temporary `$HOME` and a stripped `PATH`, and set `DEV_AUTO_UPDATE=0`.
- Anything that would download, like an installer or a release, is replaced by a fake behind a seam (the `with_*`
  methods on dependencies) or served over `file://`.
- Fake executables are written with `common::write_executable`, never `fs::write`. On Linux, running a file another
  test thread has open for writing fails with "Text file busy".

To try a real `dev up`, use `dev test:docker`. It builds an image with dev installed, the shell setup loaded, and
`testdata/dev.yaml` as the project.

## Commits and releases

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) (`feat:`, `fix:`,
`docs:`, ...), with a body that says why.

Releases are cut by running the Release workflow with a patch, minor, or major bump. It updates the version in
`Cargo.toml`, tags it, and publishes binaries for `install.sh` and `dev self-update`. Don't change the version by
hand.

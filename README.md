# dev

[![CI](https://github.com/pseudomuto/dev/actions/workflows/ci.yaml/badge.svg?branch=main)](https://github.com/pseudomuto/dev/actions/workflows/ci.yaml?query=branch%3Amain)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.99.0](https://img.shields.io/badge/rust-1.99.0-orange.svg?logo=rust)](rust-toolchain.toml)
![Platforms: macOS | Linux](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey.svg)

![dev up installing a brew package and a mise tool, env loading on cd, and a project command with flags](demo/demo.gif)

`dev` sets up a project's development environment from a `dev.yaml` at its root. Describe the tools, packages, env vars,
and commands the project needs, and anyone can get going with:

```sh
dev up
```

It's idempotent: `dev up` only does what isn't done yet, so it's safe to run any time, e.g. after pulling.

**Docs: <https://pseudomuto.com/dev>**

## Features at a glance

- **Packages** from Homebrew formulae, installed with [zerobrew](https://zerobrew.rs). See
  [deps](https://pseudomuto.com/dev/dev-yaml/deps.html).
- **Tools** like node, python, or go, installed with [mise](https://mise.jdx.dev), without touching your own mise
  config. A project's tools only apply inside it; set global ones with `dev up --global`. See
  [Global dev.yaml](https://pseudomuto.com/dev/dev-yaml/global.html).
- **Rust toolchains** with rustup, straight from `rust-toolchain.toml` or listed by channel.
- **Project packages** (npm, yarn, pnpm, bun, bundler, go, cargo, uv, poetry) installed when they're out of date, and
  strictly from the lockfile in CI.
- **Env vars**, loaded into your shell when you `cd` into the project and removed when you leave, layered from a global
  `dev.yaml` down. See [env](https://pseudomuto.com/dev/dev-yaml/env.html) and
  [How env is loaded](https://pseudomuto.com/dev/how-env-is-loaded.html).
- **Project commands**, run with `dev <name>`, with optional flags, args, and generated `--help`. See
  [commands](https://pseudomuto.com/dev/dev-yaml/commands.html).
- **New projects** from built-in templates (Go, Node, Python, Ruby, Rust) or your own, with `dev new`. See
  [New projects](https://pseudomuto.com/dev/new-projects.html).
- **Logs** of every `dev up` and `dev down` in `.dev/log`, so the terminal only shows progress. See
  [Logs](https://pseudomuto.com/dev/cli.html#logs).
- **Self-updates** in the background, checked against release checksums. See
  [Updates](https://pseudomuto.com/dev/updates.html).
- **A GitHub action** that runs `dev up` in CI and caches what it installs. See
  [GitHub Actions](https://pseudomuto.com/dev/github-actions.html).

## Install

```sh
curl -fsSL https://pseudomuto.com/dev/install.sh | sh
```

This installs the latest release for your machine (macOS or Linux, x86_64 or arm64) to `~/.local/bin/dev`. Then add the
shell setup to your rc file (`.zshrc`, `.bashrc`, ...):

```sh
eval "$(dev shell-init)"
```

See [Install](https://pseudomuto.com/dev/getting-started/install.html) for picking a version or install directory, and
building from source.

## Quick start

Add a `dev.yaml` to your project:

```yaml
name: my-app

env:
  PATH: "${PWD}/bin:${PATH}"
  RUST_LOG: debug

deps:
  brew:
    - jq
  node: lts
  python: "3.14"

commands:
  test: cargo test
```

Then:

```sh
dev up      # installs zerobrew, jq, mise, node, and python
dev test    # runs `cargo test` with the project env
```

Everything else, from every `dev.yaml` key to the GitHub action, is in the [docs](https://pseudomuto.com/dev).

## Contributing

See [CONTRIBUTING.md](.github/CONTRIBUTING.md) for setting up the repo, running the checks, and how releases work.

## License

[MIT](LICENSE)

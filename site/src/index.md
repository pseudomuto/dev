# dev

**dev is a tool for managing development environments.** Describe what a project needs in a `dev.yaml` at its root
(tools, packages, env vars, and commands), and anyone can go from a fresh clone to a working setup with one command:

```sh
dev up
```

It's idempotent: `dev up` only does what isn't done yet, so it's safe to run any time, e.g. after pulling.

![dev up installing a brew package and a mise tool, env loading on cd, and a project command with flags](demo.gif)

## Features at a glance

- **Packages** from Homebrew formulae, installed with [zerobrew](https://zerobrew.rs). See [deps](dev-yaml/deps.md).
- **Tools** like node, python, or go, installed with [mise](https://mise.jdx.dev), without touching your own mise
  config. A project's tools only apply inside it; set global ones with `dev up --global`. See
  [Global dev.yaml](dev-yaml/global.md).
- **Rust toolchains** with rustup, straight from `rust-toolchain.toml` or listed by channel.
- **Project packages** (npm, yarn, pnpm, bun, bundler, go, cargo, uv, poetry) installed when they're out of date, and
  strictly from the lockfile in CI.
- **Env vars**, loaded into your shell when you `cd` into the project and removed when you leave, layered from a global
  `dev.yaml` down. See [env](dev-yaml/env.md) and [How env is loaded](how-env-is-loaded.md).
- **Project commands**, run with `dev <name>`, with optional flags, args, and generated `--help`. See
  [commands](dev-yaml/commands.md).
- **New projects** from built-in templates (Go, Node, Python, Ruby, Rust) or your own, with `dev new`. See
  [New projects](new-projects.md).
- **Logs** of every `dev up` and `dev down` in `.dev/log`, so the terminal only shows progress. See [Logs](cli.md#logs).
- **Self-updates** in the background, checked against release checksums. See [Updates](updates.md).
- **A GitHub action** that runs `dev up` in CI and caches what it installs. See [GitHub Actions](github-actions.md).

## Where to next

- New to dev? Start with [Install](getting-started/install.md), then the [Quick start](getting-started/quick-start.md).
- Writing a `dev.yaml`? The [dev.yaml reference](dev-yaml/index.md) covers every key.
- Setting up CI? See [GitHub Actions](github-actions.md).

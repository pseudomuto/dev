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

What it handles:

- **Packages** from Homebrew formulae, installed with [zerobrew](https://zerobrew.rs).
- **Tools** like node, python, or go, installed with [mise](https://mise.jdx.dev).
- **Env vars**, loaded into your shell when you `cd` into the project and removed when you leave.
- **Project commands**, run with `dev <name>`, with optional flags, args, and generated `--help`.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/pseudomuto/dev/main/install.sh | sh
```

This installs the latest release for your machine (macOS or Linux, x86_64 or arm64) to `~/.local/bin/dev`, after
checking it against the release's checksums. It doesn't change anything else: it prints what to add to your shell's rc
file (`.zshrc`, `.bashrc`, ...), which is:

```sh
eval "$(dev shell-init)"
```

The shell setup is what makes tools available right after `dev up`, and loads project env as you move between
directories. `dev` works without it, but you'd have to open a new shell after each `dev up`. bash and zsh are supported.

To pick the version or where it goes, set these for `sh`:

```sh
curl -fsSL https://raw.githubusercontent.com/pseudomuto/dev/main/install.sh | DEV_VERSION=0.1.0 DEV_INSTALL_DIR=/usr/local/bin sh
```

dev keeps itself up to date (see [Updates](#updates)). To build from source instead (the Rust version is pinned in
`rust-toolchain.toml`):

```sh
cargo install --path .
```

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

## dev.yaml

| Key        | Required | What it is                                              |
| ---------- | -------- | ------------------------------------------------------- |
| `name`     | yes      | Lowercase words separated by dashes, e.g. `my-app`.     |
| `env`      | no       | Env vars set for the project's shell and every command. |
| `deps`     | no       | What `dev up` installs, in order.                       |
| `commands` | no       | Commands run with `dev <name>`.                         |

Unknown keys are an error, so typos don't go unnoticed.

### env

```yaml
env:
  PATH: "${PWD}/bin:${PATH}"
  DATABASE_URL: postgres://localhost/my_app
```

`${VAR}` is replaced with the var's current value, and an unset var is an error. `${PWD}` is always the directory
holding the `dev.yaml`, so `${PWD}/bin` means the same thing from any subdirectory. A bare `$VAR` is left as is.

`${VAR:-default}` uses `VAR` if it's set and not empty, and `default` otherwise. This is how a var can be overridden,
e.g. in CI:

```yaml
env:
  DATABASE_URL: "${DATABASE_URL:-postgres://localhost/my_app}"
```

Locally you get `postgres://localhost/my_app`. In CI, set `DATABASE_URL` in the job's `env:` and `dev test` uses that
instead. The default is plain text: it can't contain another `${...}`. Vars without a default always get the value in
`dev.yaml`.

### deps

```yaml
deps:
  brew:
    - jq
    - yq
  rust:
    - rust-toolchain.toml
    - nightly: [rustfmt]
  node: lts
  python: "3.14"
  shellcheck: latest
```

- `brew` installs zerobrew (or updates it to the latest release), then each package, upgrading any that are outdated.
- `rust` installs rustup if it isn't already on `PATH` (into `~/.cargo`, without touching your rc files), then each
  toolchain:
  - `rust-toolchain.toml` installs what that file names: its channel, components, and targets.
  - `nightly`, or `"1.80"` (quoted, so YAML keeps it as text), installs that toolchain.
  - `nightly: [rustfmt]` installs a toolchain with those components.

  A channel like `nightly` is installed once, not updated on every `dev up`. Everything after `rust` in the same
  `dev up` finds `cargo`, e.g. a mise `cargo:` tool.

- Every other key is a [mise](https://mise.jdx.dev) tool and its version. dev installs mise if it isn't already on
  `PATH`, and adds the tools to its own mise config (`~/.config/dev/mise.toml`), which mise loads alongside yours. Your
  own global mise config is never changed, and wins when both pin the same tool.
- `packages` controls installing the project's own packages. You usually don't need it: after everything else, dev finds
  these in the project root and installs them when they're out of date:

  | Found                     | Runs                                                 |
  | ------------------------- | ---------------------------------------------------- |
  | `package.json`            | `npm`, `yarn`, `pnpm`, or `bun install`, by lockfile |
  | `Gemfile`                 | `bundle install`                                     |
  | `go.mod`                  | `go mod download`                                    |
  | `Cargo.toml`              | `cargo fetch`                                        |
  | `uv.lock` / `poetry.lock` | `uv sync` / `poetry install`                         |

  "Out of date" means the manifest, lockfile, or runtime version (e.g. `node --version`) changed since the last install,
  or `node_modules` / `.venv` is gone. The runtime has to be on `PATH`, e.g. `node: lts` under `deps`. `packages: false`
  turns this off; `packages: [node, ruby]` installs only those, at that spot in `deps`.

  When `CI` is set (as CI services do), installs use the lockfile as is and fail if it's out of date: `npm ci`,
  `--frozen-lockfile` for yarn, pnpm, and bun, `BUNDLE_FROZEN=true`, `cargo fetch --locked`, and `uv sync --locked`. An
  install that changes the manifest or lockfile anyway (e.g. `go mod download` filling in `go.sum`) fails, since it
  means they weren't committed up to date.

- `custom` is reserved for a future built-in dependency.

### commands

The short form is a shell snippet run with `sh -c`. Args are passed through, so `$@` has them and `$0` is the command's
name:

```yaml
commands:
  test: cargo test "$@"
  lint: |
    cargo fmt --check
    cargo clippy -- -D warnings
```

The long form is a map. It runs either a `run` snippet or a script `file`, and can have a `desc`, `flags`, and `args`:

```yaml
commands:
  deploy:
    desc: Deploy an app
    flags:
      env: { short: e, default: staging, desc: Target environment }
      dry-run: { type: bool, desc: Print what would happen }
    args:
      - name: target
        desc: App to deploy
    run: |
      echo "Deploying $TARGET to $ENV"
      [ "$DRY_RUN" = true ] && exit 0
      ./scripts/deploy "$TARGET" "$ENV"

  release:
    file: scripts/release.sh
    flags:
      yes: { short: y, type: bool }
```

```sh
dev deploy web -e prod --dry-run
dev deploy --help
dev release -y -- --notes CHANGELOG.md
```

How the long form works:

- **`run` or `file`**, exactly one. A `file` path is relative to `dev.yaml` and must be executable. It runs directly, so
  its shebang picks the interpreter (bash, python, ...).
- **Flags and args become env vars**, uppercased with dashes turned into underscores: `--dry-run` is `$DRY_RUN`. This
  works the same for `run` and `file`.
- **Flag `type`** is `string` (the default) or `bool`. Bool flags are always `true` or `false`. A string flag with no
  default that isn't passed is left unset.
- **Args** are positional, in the order listed. They're required unless they have a `default` or `required: false`, and
  required ones can't come after optional ones.
- **`--help`** is generated from `desc`, `flags`, and `args`. Bad input prints usage and exits with status 2.
- **Anything after `--`** is passed to the script as its own args (`$@`).
- **Names** of flags and args are lowercase words separated by dashes. A few are reserved because their env var would
  break the script: `path`, `home`, `pwd`, `ifs`, `shell`, and `help`. `-h` is taken by `--help`.

A long-form command without `flags` or `args` passes its args through as is, like the short form.

Command names can't match a built-in command (`up`, `down`, `self-update`, `shell-init`, `hook-env`, `help`). Commands
always exit with their own status.

## Commands

| Command            | What it does                                                             |
| ------------------ | ------------------------------------------------------------------------ |
| `dev up`           | Installs or updates every dependency in `./dev.yaml` that isn't current. |
| `dev down`         | Takes down every dependency, last one first.                             |
| `dev self-update`  | Installs the latest release, if it's newer.                              |
| `dev shell-init`   | Prints the shell setup, for `eval "$(dev shell-init)"` in your rc file.  |
| `dev <name> [...]` | Runs a command from the `commands` section.                              |

`dev --help` lists the project's commands under the built-in ones, with their `desc`.

`dev down` currently leaves zerobrew, mise, and what they installed in place, since other projects may use them.

On a terminal, `dev up` and `dev down` show live progress. Set `DEV_UI=plain` for plain lines instead, which is also
what you get when the output isn't a terminal (e.g. in CI).

### Logs

The full output of every `dev up` and `dev down` goes to `.dev/log` next to `dev.yaml`, so the terminal only shows
progress. The last 5 runs are kept, and `.dev/log/latest` points at the newest:

```sh
less .dev/log/latest
```

When a step fails, dev shows its last lines of output and where the full log is. `.dev` has its own `.gitignore`, so
there's nothing to add to yours.

## Updates

With the shell setup loaded, dev checks for a new release at most every 4 hours, from the prompt. The check and the
install run in the background, so the prompt never waits on them. The next `dev` command then says what changed:

```text
dev updated 0.1.0 -> 0.2.0
```

`dev self-update` does the same on demand. Installs are checked against the release's checksums, the same as the
installer.

- Set `DEV_AUTO_UPDATE=0` to turn it off, e.g. in your rc file.
- Builds from source (debug builds, and `cargo install` into `~/.cargo/bin`) don't update themselves.
- The last background run's output is in `~/.local/state/dev/update.log` (or `$XDG_STATE_HOME/dev`), for when one fails.

## GitHub Actions

This repo is also a GitHub action. Put it right after checkout, and later steps have everything in your `dev.yaml`:

```yaml
steps:
  - uses: actions/checkout@<sha> # v7.0.1
  - uses: pseudomuto/dev@<sha> # v0.2.0
  - run: dev test
```

It installs dev, runs `dev up` from the root of the checkout, and puts the tools it installed on `PATH`, so plain `run:`
steps like `cargo test` or `jq` work too. `dev up` also installs your project's packages, from the lockfile as is (see
[deps](#deps)), so you can drop your own `npm ci` or `bundle install` step. Pin it to a commit SHA, as above, or to a
release tag: either way it installs the matching dev release. Dependabot and Renovate can keep the pin up to date. On
Linux and macOS only.

| Input             | Default | What it does                                                                      |
| ----------------- | ------- | --------------------------------------------------------------------------------- |
| `version`         |         | dev version to install, e.g. `0.2.0`. Needed when the action is used at a branch. |
| `cache`           | `true`  | Set to `false` to skip the cache.                                                 |
| `cache-key-files` |         | Extra files whose changes should refresh the cache, one glob per line.            |
| `export-env`      | `false` | Also export `dev.yaml`'s `env` to later steps (see below).                        |

| Output      | What it is                                 |
| ----------- | ------------------------------------------ |
| `version`   | The dev version installed.                 |
| `cache-hit` | `true` when the cache key matched exactly. |

### Caching

The action caches what `dev up` installs: Rust toolchains, mise tools, and brew packages. The cache refreshes when
`dev.yaml`, `rust-toolchain.toml`, or a `cache-key-files` match changes; otherwise `dev up` only installs what's
missing. A few things to know:

- **It doesn't cache your project's packages or build output** (`~/.npm`, `~/.cargo/registry`, `target/`). Keep using
  your language's cache action for those, before this one, so `dev up` installs packages from a warm cache:

  ```yaml
  - uses: actions/checkout@<sha> # v7.0.1
  - uses: actions/cache@<sha> # v6.1.0
    with:
      path: ~/.npm
      key: npm-${{ runner.os }}-${{ hashFiles('package-lock.json') }}
  - uses: Swatinem/rust-cache@<sha> # v2.9.3
  - uses: pseudomuto/dev@<sha> # v0.2.0
  - run: dev test
  ```

- **Ruby gems end up in this cache.** `bundle install` puts them inside the Ruby that mise installed, which is cached
  with the tools. That only speeds things up (bundler still installs what `Gemfile.lock` says), but old gems stay until
  the key changes.
- **Pin versions in `dev.yaml`.** With `latest` or `lts`, a new upstream release is downloaded on every run until
  `dev.yaml` changes, since the cache can't tell it's new.
- **Brew packages are cached on their own and only reused on an exact key match.** dev never uninstalls brew packages,
  so this keeps one you removed from `dev.yaml` from lingering in CI. The cost: any change to the key reinstalls them.
- **Old Rust toolchains stay in the cache** after you move to a new one. If it gets big, change `cache-key-files` (or
  anything else in the key) to start a fresh one. Unused mise tool versions are dropped automatically.

### Env in CI

`dev <name>` always applies `dev.yaml`'s `env`, so `run: dev test` gets it with no setup. For steps that run tools
directly, set `export-env: true` and the vars are exported to every later step, with values as of the action's step.

To use a different value in CI, give the var a default in `dev.yaml` with `${VAR:-default}`, and set it in the job:

```yaml
# dev.yaml
env:
  DATABASE_URL: "${DATABASE_URL:-postgres://localhost/my_app}"
```

```yaml
# .github/workflows/ci.yaml
jobs:
  test:
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:17
        env:
          POSTGRES_PASSWORD: postgres
        ports: ["5432:5432"]
    env:
      DATABASE_URL: postgres://postgres:postgres@localhost:5432/my_app
    steps:
      - uses: actions/checkout@<sha> # v7.0.1
      - uses: pseudomuto/dev@<sha> # v0.2.0
      - run: dev test # uses the job's DATABASE_URL
```

## How env is loaded

With the shell setup loaded, your shell picks up `env` from every `dev.yaml` that applies to the current directory,
before each prompt. They're layered, outermost first:

1. `~/.config/dev.yaml` (or `$XDG_CONFIG_HOME/dev.yaml`), which applies everywhere.
2. Each `dev.yaml` from just below `$HOME` down to the current directory.

Outside `$HOME`, only `./dev.yaml` is checked. Later layers override earlier ones, and `${VAR}` in a layer sees what
earlier layers set. When you leave a directory, the vars it set go back to what they were before. Editing a `dev.yaml`
takes effect at the next prompt.

`dev <name>` and `dev up` apply `./dev.yaml`'s env themselves, so they work without the shell setup too.

## Contributing

See [CONTRIBUTING.md](.github/CONTRIBUTING.md) for setting up the repo, running the checks, and how releases work.

## License

[MIT](LICENSE)

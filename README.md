# dev

[![CI](https://github.com/pseudomuto/dev/actions/workflows/ci.yaml/badge.svg?branch=main)](https://github.com/pseudomuto/dev/actions/workflows/ci.yaml?query=branch%3Amain)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.99.0](https://img.shields.io/badge/rust-1.99.0-orange.svg?logo=rust)](rust-toolchain.toml)
![Platforms: macOS | Linux](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey.svg)

`dev` sets up a project's development environment from a `dev.yaml` at its root. Describe the tools, packages, env
vars, and commands the project needs, and anyone can get going with:

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

Build it from source (needs Rust; the version is pinned in `rust-toolchain.toml`):

```sh
cargo install --path .
```

Then copy the shell setup somewhere and source it from your shell's rc file (`.zshrc`, `.bashrc`, ...):

```sh
sudo mkdir -p /usr/local/share/dev
sudo cp share/shell /usr/local/share/dev/shell
echo '. /usr/local/share/dev/shell' >> ~/.zshrc
```

The shell setup is what makes tools available right after `dev up`, and loads project env as you move between
directories. `dev` works without it, but you'd have to open a new shell after each `dev up`. bash and zsh are
supported.

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

| Key        | Required | What it is                                                     |
| ---------- | -------- | -------------------------------------------------------------- |
| `name`     | yes      | Lowercase words separated by dashes, e.g. `my-app`.            |
| `env`      | no       | Env vars set for the project's shell and every command.        |
| `deps`     | no       | What `dev up` installs, in order.                              |
| `commands` | no       | Commands run with `dev <name>`.                                |

Unknown keys are an error, so typos don't go unnoticed.

### env

```yaml
env:
  PATH: "${PWD}/bin:${PATH}"
  DATABASE_URL: postgres://localhost/my_app
```

`${VAR}` is replaced with the var's current value, and an unset var is an error. `${PWD}` is always the directory
holding the `dev.yaml`, so `${PWD}/bin` means the same thing from any subdirectory. A bare `$VAR` is left as is.

### deps

```yaml
deps:
  brew:
    - jq
    - yq
  node: lts
  python: "3.14"
  shellcheck: latest
```

- `brew` installs zerobrew (or updates it to the latest release), then each package, upgrading any that are outdated.
- Every other key is a [mise](https://mise.jdx.dev) tool and its version. dev installs mise if it isn't already on
  `PATH`, and adds the tools to its own mise config (`~/.config/dev/mise.toml`), which mise loads alongside yours. Your
  own global mise config is never changed, and wins when both pin the same tool.
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

- **`run` or `file`**, exactly one. A `file` path is relative to `dev.yaml` and must be executable. It runs directly,
  so its shebang picks the interpreter (bash, python, ...).
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

Command names can't match a built-in command (`up`, `down`, `help`). Commands always exit with their own status.

## Commands

| Command            | What it does                                                              |
| ------------------ | ------------------------------------------------------------------------- |
| `dev up`           | Installs or updates every dependency in `./dev.yaml` that isn't current.  |
| `dev down`         | Takes down every dependency, last one first.                              |
| `dev <name> [...]` | Runs a command from the `commands` section.                               |

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

## How env is loaded

With the shell setup sourced, your shell picks up `env` from every `dev.yaml` that applies to the current directory,
before each prompt. They're layered, outermost first:

1. `~/.config/dev.yaml` (or `$XDG_CONFIG_HOME/dev.yaml`), which applies everywhere.
2. Each `dev.yaml` from just below `$HOME` down to the current directory.

Outside `$HOME`, only `./dev.yaml` is checked. Later layers override earlier ones, and `${VAR}` in a layer sees what
earlier layers set. When you leave a directory, the vars it set go back to what they were before. Editing a `dev.yaml`
takes effect at the next prompt.

`dev <name>` and `dev up` apply `./dev.yaml`'s env themselves, so they work without the shell setup too.

## Development

Tasks run with [mise](https://mise.jdx.dev):

| Task                   | What it does                                     |
| ---------------------- | ------------------------------------------------ |
| `mise run build`       | Builds a debug binary.                           |
| `mise run test`        | Runs the tests.                                  |
| `mise run lint`        | Runs clippy, with warnings as errors.            |
| `mise run fmt`         | Formats the code (needs nightly rustfmt).        |
| `mise run test:ci`     | Runs everything CI does: format, lints, tests.   |
| `mise run test:docker` | Opens a shell in a throwaway Linux container.    |

`test:docker` builds an image with dev installed, the shell setup sourced, and `testdata/dev.yaml` as the project, so
you can try `dev up` for real without touching your machine.

Tests never touch the network or your machine's setup: they run `dev` with a temporary `$HOME` and a stripped
`PATH`.

## License

[MIT](LICENSE)

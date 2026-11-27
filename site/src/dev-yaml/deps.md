# deps

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
  `PATH`, and writes the tools to the project's own mise config, `.dev/mise.toml`. The project's env sets
  `MISE_DEFAULT_CONFIG_FILENAME=.dev/mise.toml`, which makes mise look for that file next to its usual ones, so the
  tools apply inside the project and nowhere else. Your own mise configs are never changed. In the project, dev's file
  wins over its `mise.toml`, and loses to `.mise.toml` and `mise.local.toml`.

  To pass [tool options](https://mise.jdx.dev/dev-tools/#tool-options), use a map with a `version`:

  ```yaml
  deps:
    aws: { version: "2.37", symlink_bins: true }
  ```

  Option values are plain strings, numbers, or bools, and can't contain `[`, `]`, `,`, or `=`. dev also passes them when
  installing (`mise install aws[symlink_bins=true]@2.37`), for options that change how a tool is installed. mise only
  loads a project's config with options once it's trusted, so `dev up` runs `mise trust` on `.dev/mise.toml` then; it
  installs with those options anyway.

  dev rewrites `.dev/mise.toml` on every `dev up` to match `dev.yaml`, so don't edit it. That includes `mise use`: in a
  project with no `mise.toml`, it writes to `.dev/mise.toml`, and the next `dev up` drops it. Add a `mise.toml` first,
  or put the tool in `dev.yaml`.

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
  turns this off; `packages: [node, ruby]` installs only those, at that spot in `deps`. `dev up` fails if one of them
  has none of its files, though the project's env and commands still work.

  When `CI` is set (as CI services do), installs use the lockfile as is and fail if it's out of date: `npm ci`,
  `--frozen-lockfile` for yarn, pnpm, and bun, `BUNDLE_FROZEN=true`, `cargo fetch --locked`, and `uv sync --locked`. An
  install that changes the manifest or lockfile anyway (e.g. `go mod download` filling in `go.sum`) fails, since it
  means they weren't committed up to date.

- `custom` is a list of shell checks, for anything the other deps don't cover:

  ```yaml
  deps:
    custom:
      - name: Ensure file exists
        met: test -f test-file.txt
        meet: echo "yo yo yo" > test-file.txt
        down: rm -f test-file.txt
  ```

  `dev up` shows each one by its `name`. When `met` exits 0 it's met and nothing else runs. Otherwise dev runs `meet`,
  then `met` again, which has to pass this time. `dev down` runs `down`, if it's set. Every snippet runs with `sh -c`,
  like a command, in the project root (`HOME` for the global file), with the project's env and no stdin. A multi-line
  `met` only reports its last line's exit status, so join the checks with `&&`, start it with `set -e`, or make it a
  [list](lists.md). dev waits until every process a snippet starts closes its output, so send a background process's
  output elsewhere (`./start-server >/dev/null 2>&1 &`), or `dev up` waits for it to exit.

# GitHub Actions

This repo is also a GitHub action. Put it right after checkout, and later steps have everything in your `dev.yaml`:

```yaml
steps:
  - uses: actions/checkout@<sha> # v7.0.1
  - uses: pseudomuto/dev@<sha> # v0.2.0
  - run: dev test
```

It installs dev, runs `dev up` from the root of the checkout, and puts the tools it installed on `PATH`, so plain `run:`
steps like `cargo test` or `jq` work too. `dev up` also installs your project's packages, from the lockfile as is (see
[deps](dev-yaml/deps.md)), so you can drop your own `npm ci` or `bundle install` step. Pin it to a commit SHA, as above,
or to a release tag: either way it installs the matching dev release. Dependabot and Renovate can keep the pin up to
date. On Linux and macOS only. It needs dev 0.4.0 or later, so a `version` set to an older release fails before
installing anything.

| Input             | Default | What it does                                                                      |
| ----------------- | ------- | --------------------------------------------------------------------------------- |
| `version`         |         | dev version to install, e.g. `0.4.0`. Needed when the action is used at a branch. |
| `cache`           | `true`  | Set to `false` to skip the cache.                                                 |
| `cache-key-files` |         | Extra files whose changes should refresh the cache, one glob per line.            |
| `export-env`      | `false` | Also export `dev.yaml`'s `env` to later steps (see below).                        |

| Output      | What it is                                 |
| ----------- | ------------------------------------------ |
| `version`   | The dev version installed.                 |
| `cache-hit` | `true` when the cache key matched exactly. |

## Caching

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
- **Brew packages aren't cached on macOS.** zerobrew keeps them in `/opt/zerobrew` there, and restoring into root-owned
  `/opt` would need sudo, which the action doesn't use. `dev up` still installs them.
- **The cached paths come from dev**, so they follow the runner's `XDG_DATA_HOME`, `MISE_DATA_DIR`, `MISE_INSTALL_PATH`,
  `RUSTUP_HOME`, `ZEROBREW_BIN`, and `ZEROBREW_ROOT`, e.g. on a self-hosted runner.
- **Old Rust toolchains stay in the cache** after you move to a new one. If it gets big, change `cache-key-files` (or
  anything else in the key) to start a fresh one. Unused mise tool versions are dropped automatically.

## Env in CI

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

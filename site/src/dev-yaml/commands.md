# commands

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

- **`run` or `file`**, exactly one, unless the command has [`depends`](depends.md), where both are optional. A `file`
  path is relative to `dev.yaml` and must be executable. It runs directly, so its shebang picks the interpreter (bash,
  python, ...).
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

Command names can't match a built-in command (`up`, `down`, `new`, `self-update`, `shell-init`, `hook-env`, `help`).
Commands always exit with their own status.

# Running other commands

In a command's list (not a custom dep's), an item can run another command from `dev.yaml`, without starting dev again:

```yaml
commands:
  test:
    flags:
      verbose: { type: bool }
    run: cargo test
  check:
    - cargo fmt --check
    - { cmd: test, args: [--verbose], env: { RUST_LOG: debug } }
    - echo all good
```

- `cmd` is the command's name. `args` and `env` are optional.
- `args` is its command line, as in `dev test --verbose`, parsed by its own `flags` and `args`. It doesn't get the
  caller's `$@`.
- Its env is the caller's (including the caller's flag and arg vars), then its own flag and arg vars, then `env`, each
  overriding the last. So a bool flag the caller passed is `false` again unless `args` passes it too.
- Values in `args` and `env` are used as is, with no `$VAR` expansion.
- The snippets around a `cmd` item run as separate scripts, so `cd` and vars don't carry across it.
- The list stops at the first step that fails, with its exit code. A `cmd` naming a command that doesn't exist, or a
  command that ends up running itself (`a -> b -> a`), is an error when dev.yaml loads.

# Depends

A long-form command can list other commands to run before it:

```yaml
commands:
  ci:
    desc: Run everything CI runs
    depends: [fmt:check, lint, test]
  release:
    depends: [ci]
    run: ./scripts/release.sh
```

- **Order:** deps run one at a time, before the command, in the order listed. A dep's own deps run before it, and a
  command that several deps share runs once.
- **Failures:** the first dep that fails stops everything, and dev exits with its status.
- **Args:** deps get none, so their flags take their defaults. What you type goes only to the command you ran, and a
  command with no `run` or `file` can't declare flags or args, and takes none.
- **Output:** dev prints `==> <dep>` before each dep, so you can tell them apart.
- **`cmd` items:** a command run by a [`{ cmd: ... }` item](running-commands.md) runs its deps too.
- **Checks:** dev.yaml fails to load if a dep doesn't exist, if a dep has a required arg, or if a command ends up
  running itself through `depends` or `cmd` items.

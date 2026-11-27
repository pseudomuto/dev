# Lists of snippets

Anything that takes a shell snippet (a short-form command, `run`, and a custom dep's `met`, `meet`, and `down`) also
takes a list of them:

```yaml
commands:
  check:
    - cargo fmt --check
    - |
      cd crates/core
      cargo test "$@"
```

The items run as one script with `set -e`, like a CI job's steps: in order, stopping at the first one that fails. Each
item gets the same `$0` and `$@`, and `cd` or vars set in one carry over to the next. Items can be multi-line strings,
but not lists or maps. dev doesn't set `pipefail`, since not every `sh` has it, so a failure early in a pipe
(`false | true`) doesn't stop the script.

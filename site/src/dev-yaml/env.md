# env

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

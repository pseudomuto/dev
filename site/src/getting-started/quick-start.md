# Quick start

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

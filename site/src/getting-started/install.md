# Install

```sh
curl -fsSL https://pseudomuto.com/dev/install.sh | sh
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
curl -fsSL https://pseudomuto.com/dev/install.sh | DEV_VERSION=0.1.0 DEV_INSTALL_DIR=/usr/local/bin sh
```

dev keeps itself up to date (see [Updates](../updates.md)). To build from source instead (the Rust version is pinned in
`rust-toolchain.toml`):

```sh
cargo install --path .
```

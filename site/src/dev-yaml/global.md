# Global dev.yaml

`~/.config/dev.yaml` (or `$XDG_CONFIG_HOME/dev.yaml`) is yours, for what applies everywhere. Its `env` is loaded in
every shell (see [How env is loaded](../how-env-is-loaded.md)), and `dev up --global` installs its `deps`, from any
directory:

```yaml
env:
  EDITOR: nvim
deps:
  node: "24"
  go: "1.25"
  "go:golang.org/x/tools/gopls": latest
```

It works like a project's `dev.yaml`, except:

- `name` is optional.
- Tools go to dev's global mise config, `~/.config/dev/mise.toml`, which mise loads everywhere through a link at
  `~/.config/mise/conf.d/dev.toml`. Your own global mise config wins when both pin the same tool. Like a project's, dev
  rewrites this file on every `dev up --global` to match `dev.yaml`.
- Its deps run in `$HOME`, wherever you run `dev up --global`, so a `rust-toolchain.toml` entry reads
  `~/rust-toolchain.toml`. `${PWD}` in it is `$HOME` too.
- Project packages aren't detected next to it, and `commands` aren't run by `dev <name>`.

# New projects

```sh
dev new my-app -t go
```

This creates `./my-app` from the `go` template, runs `git init` in it, and then `dev up`, so it's ready to work in. The
name follows the same rule as `dev.yaml`'s `name`, and the directory must not exist yet.

Built-in templates:

| Template | What you get                                                                        |
| -------- | ----------------------------------------------------------------------------------- |
| `go`     | `dev.yaml` with Go, `go.mod`, `main.go`, commands to build, run, and test, and CI.  |
| `node`   | TypeScript that Node runs as is, `node --test`, ESLint, Prettier, commands, and CI. |
| `python` | uv with pytest and Ruff, `main.py`, a `uv.lock` so `dev up` installs them, and CI.  |
| `ruby`   | Bundler with Minitest and RuboCop, `main.rb`, a `Rakefile`, commands, and CI.       |
| `rust`   | `dev.yaml`, `Cargo.toml`, `rust-toolchain.toml`, `src/main.rs`, commands, and CI.   |

The CI workflow (`.github/workflows/ci.yaml`) runs `dev lint` and `dev test` on pull requests and pushes to `main`,
using [the GitHub action](github-actions.md) pinned to the release of dev that created the project.

Rust (to link) and Ruby (to build gems like `json`) need a C compiler, which `dev up` doesn't install: the Xcode Command
Line Tools on macOS (`xcode-select --install`), or e.g. `build-essential` on Debian and Ubuntu. On Linux, Node also
needs `libatomic1`, which slim images leave out. GitHub's runners have all of these.

To add your own, put a directory in `~/.config/dev/templates` (or `$XDG_CONFIG_HOME/dev/templates`), e.g.
`~/.config/dev/templates/api`, and use it with `-t api`. One with the same name as a built-in template replaces it.
`{{name}}` in file contents and paths is replaced with the project's name, and `{{dev_version}}` with the ref to use
this repo's action at (e.g. `uses: pseudomuto/dev@{{dev_version}}`): the release's commit with its version as a comment,
or the version's tag for a dev built from source. Executable files stay executable. A `.dev-notes.txt` at the template's
root isn't copied: it's rendered the same way and printed once the project is created, e.g. to say what to change first.

If `dev up` fails, the project is still created: fix it and run `dev up` from inside it.

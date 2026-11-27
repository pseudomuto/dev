# Commands

| Command            | What it does                                                             |
| ------------------ | ------------------------------------------------------------------------ |
| `dev up`           | Installs or updates every dependency in `./dev.yaml` that isn't current. |
| `dev up --global`  | Same, for the [global dev.yaml](dev-yaml/global.md), from any directory. |
| `dev down`         | Takes down every dependency, last one first.                             |
| `dev new <name>`   | Creates a project from a template. See [New projects](new-projects.md).  |
| `dev self-update`  | Installs the latest release, if it's newer.                              |
| `dev shell-init`   | Prints the shell setup, for `eval "$(dev shell-init)"` in your rc file.  |
| `dev <name> [...]` | Runs a command from the `commands` section.                              |

`dev --help` lists the project's commands under the built-in ones, with their `desc`.

`dev down` currently leaves zerobrew, mise, and what they installed in place, since other projects may use them.

On a terminal, `dev up` and `dev down` show live progress. Set `DEV_UI=plain` for plain lines instead, which is also
what you get when the output isn't a terminal (e.g. in CI).

## Logs

The full output of every `dev up` and `dev down` goes to `.dev/log` next to `dev.yaml`, so the terminal only shows
progress. The last 5 runs are kept, and `.dev/log/latest` points at the newest:

```sh
less .dev/log/latest
```

When a step fails, dev shows its last lines of output and where the full log is. `.dev` has its own `.gitignore`, so
there's nothing to add to yours.

`dev up --global` logs to `~/.local/state/dev/log` (or `$XDG_STATE_HOME/dev/log`) instead.

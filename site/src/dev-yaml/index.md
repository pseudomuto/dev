# dev.yaml

| Key        | Required    | What it is                                              |
| ---------- | ----------- | ------------------------------------------------------- |
| `name`     | in projects | Lowercase words separated by dashes, e.g. `my-app`.     |
| `env`      | no          | Env vars set for the project's shell and every command. |
| `deps`     | no          | What `dev up` installs, in order.                       |
| `commands` | no          | Commands run with `dev <name>`.                         |

Unknown keys are an error, so typos don't go unnoticed. The [global dev.yaml](global.md) needs no `name`.

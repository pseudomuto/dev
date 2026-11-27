# How env is loaded

With the shell setup loaded, your shell picks up `env` from every `dev.yaml` that applies to the current directory,
before each prompt. They're layered, outermost first:

1. `~/.config/dev.yaml` (or `$XDG_CONFIG_HOME/dev.yaml`), which applies everywhere.
2. Each `dev.yaml` from just below `$HOME` down to the current directory.

Outside `$HOME`, only `./dev.yaml` is checked. Later layers override earlier ones, and `${VAR}` in a layer sees what
earlier layers set. When you leave a directory, the vars it set go back to what they were before. Editing a `dev.yaml`
takes effect at the next prompt.

`dev <name>` and `dev up` apply `./dev.yaml`'s env themselves, so they work without the shell setup too.

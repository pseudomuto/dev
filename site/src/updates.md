# Updates

With the shell setup loaded, dev checks for a new release at most every 4 hours, from the prompt. The check and the
install run in the background, so the prompt never waits on them. The next `dev` command then says what changed:

```text
dev updated 0.1.0 -> 0.2.0
```

`dev self-update` does the same on demand. Installs are checked against the release's checksums, the same as the
installer.

- Set `DEV_AUTO_UPDATE=0` to turn it off, e.g. in your rc file.
- Builds from source (debug builds, and `cargo install` into `~/.cargo/bin`) don't update themselves.
- The last background run's output is in `~/.local/state/dev/update.log` (or `$XDG_STATE_HOME/dev`), for when one fails.

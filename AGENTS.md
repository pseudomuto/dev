# Agent notes

## Keep the docs up to date

The docs site lives in `site/src/` and is published to <https://pseudomuto.com/dev>.

- A change to behavior, `dev.yaml` keys, commands, flags, or the GitHub action updates the matching pages under
  `site/src/` in the same PR.
- If the change touches the pitch, quick start, or install, update `README.md` too.
- Check the site builds with `dev docs:build`. Preview it with `dev docs`.

## Everything else

- [.github/CONTRIBUTING.md](.github/CONTRIBUTING.md) covers setup, commands, tests, and PR conventions.
- Run `dev test:ci` before opening a PR. It runs what CI runs, including the docs build.

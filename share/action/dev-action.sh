#!/bin/sh
# shellcheck shell=sh
#
# The logic behind action.yaml, the dev GitHub action. Each step of the
# action runs one subcommand:
#
#   resolve-version   Works out which dev to install.
#   cache-key         Builds the cache key from what `dev up` reads.
#
# Settings come in as env vars, and results go to the files GitHub gives each
# step ($GITHUB_OUTPUT, $GITHUB_PATH, $GITHUB_ENV). Every failure says what to
# do about it and exits non-zero: nothing falls back quietly.

set -eu

# Everything runs from here, so a truncated file runs nothing at all.
main() {
  case "${1:-}" in
    resolve-version) resolve_version ;;
    cache-key) cache_key ;;
    *) die "unknown subcommand '${1:-}'; expected resolve-version, cache-key, path, or export-env" 2 ;;
  esac
}

# Writes `version` (e.g. 0.2.0) and `installed` (true when the dev already on
# PATH is the one to use) to $GITHUB_OUTPUT. In order:
#
# 1. INPUT_VERSION, the action's `version` input. `installed` means use the
#    dev already on PATH (this repo's CI, which builds dev from the PR).
# 2. The ref the action was used at: DEV_ACTION_REF, or the last part of
#    GITHUB_ACTION_PATH, where GitHub checks the action out.
# 3. If that ref is a commit SHA (the usual way to pin an action), the release
#    tag on that commit in DEV_ACTION_REPO.
resolve_version() {
  installed=false
  if [ "${INPUT_VERSION:-}" = installed ]; then
    command -v dev >/dev/null 2>&1 || die "version is 'installed' but there's no dev on PATH"
    # `dev --version` prints e.g. `dev 0.1.0`.
    version=$(dev --version | awk '{ print $2 }')
    version=$(version_from "$version") || die "couldn't read the version of $(command -v dev)"
    installed=true
  elif [ -n "${INPUT_VERSION:-}" ]; then
    version=$(version_from "${INPUT_VERSION}") ||
      die "version '${INPUT_VERSION}' isn't a version like 0.2.0"
  else
    ref=$(action_ref)
    if is_sha "$ref"; then
      version=$(version_of_sha "$ref")
    else
      version=$(version_from "$ref") ||
        die "can't tell which dev to install from ref '$ref'; set the version input, e.g. version: 0.2.0"
    fi
  fi

  output version "$version"
  output installed "$installed"
}

# The ref the action was used at, e.g. v0.2.0, or empty if it's unknown.
action_ref() {
  if [ -n "${DEV_ACTION_REF:-}" ]; then
    echo "$DEV_ACTION_REF"
    return
  fi
  case "${GITHUB_ACTION_PATH:-}" in
    */_actions/*) echo "${GITHUB_ACTION_PATH##*/}" ;;
    *) echo "" ;;
  esac
}

# Whether $1 is a full commit SHA.
is_sha() {
  [ "${#1}" = 40 ] || return 1
  case "$1" in
    *[!0-9a-f]*) return 1 ;;
  esac
}

# Prints the version of the release tagged on commit $1, or fails if there
# isn't exactly one.
version_of_sha() {
  repo=${DEV_ACTION_REPO:-https://github.com/pseudomuto/dev.git}
  refs=$(git ls-remote --tags "$repo") || die "couldn't list the tags in $repo"

  # Annotated tags (what releases use) show up twice: the tag object, and the
  # commit it points at with a `^{}` suffix. Lightweight tags show up once,
  # as the commit.
  tags=$(printf '%s\n' "$refs" | awk -v sha="$1" '$1 == sha { sub("^refs/tags/", "", $2); sub("\\^\\{\\}$", "", $2); print $2 }' | sort -u)
  count=0
  found=
  for tag in $tags; do
    if v=$(version_from "$tag"); then
      count=$((count + 1))
      found="${found:+$found, }$v"
    fi
  done

  case $count in
    0) die "commit $1 has no release tag; set the version input, e.g. version: 0.2.0" ;;
    1) echo "$found" ;;
    *) die "commit $1 has more than one release tag ($found); set the version input to pick one" ;;
  esac
}

# Prints $1 without a leading `v` if it's a version like 0.2.0, or fails.
version_from() {
  v=${1#v}
  case "$v" in
    "" | *[!0-9.]* | .* | *. | *..*) return 1 ;;
  esac
  # Exactly three parts.
  [ "$(echo "$v" | tr -cd . | wc -c | tr -d ' ')" = 2 ] || return 1
  echo "$v"
}

# Writes `prefix` (dev-<os>-<arch>-<version>-) and `key` (the prefix and a
# hash) to $GITHUB_OUTPUT. Runs in the workspace. The hash covers dev.yaml,
# rust-toolchain.toml if there is one, and the tracked files matching each
# line of INPUT_CACHE_KEY_FILES. Language manifests are left out on purpose:
# they don't change what `dev up` installs.
cache_key() {
  [ -f dev.yaml ] || die "no dev.yaml at $(pwd); the action runs \`dev up\` from the root of the checkout"

  list=$(mktemp)
  trap 'rm -f "$list"' EXIT
  echo dev.yaml >>"$list"
  if [ -f rust-toolchain.toml ]; then
    echo rust-toolchain.toml >>"$list"
  fi

  printf '%s\n' "${INPUT_CACHE_KEY_FILES:-}" | while IFS= read -r pattern; do
    pattern=$(printf '%s' "$pattern" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')
    [ -n "$pattern" ] || continue
    # `:(glob)` makes `**/go.sum` match at any depth, like hashFiles.
    matches=$(git ls-files -- ":(glob)$pattern") ||
      die "couldn't list the files matching cache-key-files pattern '$pattern'; is $(pwd) a git checkout?"
    [ -n "$matches" ] || die "cache-key-files pattern '$pattern' matched no tracked files"
    printf '%s\n' "$matches" >>"$list"
  done

  # A line per file, path included, so a rename changes the key too.
  hash=$(sort -u "$list" | while IFS= read -r file; do
    printf '%s  %s\n' "$(sha256 <"$file")" "$file"
  done | sha256)

  os=$(printf '%s' "${DEV_ACTION_OS:?}" | tr '[:upper:]' '[:lower:]')
  arch=$(printf '%s' "${DEV_ACTION_ARCH:?}" | tr '[:upper:]' '[:lower:]')
  prefix="dev-$os-$arch-${DEV_VERSION:?}-"
  output prefix "$prefix"
  output key "$prefix$hash"
}

# Prints the SHA-256 of stdin, in hex.
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | awk '{ print $1 }'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 | awk '{ print $1 }'
  else
    die "\`sha256sum\` or \`shasum\` is required to build the cache key"
  fi
}

# Adds `$1=$2` to the step's outputs.
output() {
  printf '%s=%s\n' "$1" "$2" >>"$GITHUB_OUTPUT"
}

say() {
  printf '%s\n' "$1"
}

# Prints `error: $1` and exits with $2 (default 1).
die() {
  printf 'error: %s\n' "$1" >&2
  exit "${2:-1}"
}

main "$@"

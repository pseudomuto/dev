#!/bin/sh
# shellcheck shell=sh
#
# The logic behind action.yaml, the dev GitHub action. Each step of the
# action runs one subcommand:
#
#   resolve-version   Works out which dev to install.
#   cache-key         Builds the cache key from what `dev up` reads.
#   path              Puts the tools `dev up` installed on PATH.
#   export-env        Exports dev.yaml's env to later steps.
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
    path) path ;;
    export-env) export_env ;;
    # Internal: run by `path` in a separate shell.
    load-setup) load_setup "$2" ;;
    *) die "unknown subcommand '${1:-}'; expected resolve-version, cache-key, path, or export-env" 2 ;;
  esac
}

# Writes `version` (e.g. 0.2.0) and `installed` (true when the dev already on
# PATH is the one to use) to $GITHUB_OUTPUT. In order:
#
# 1. INPUT_VERSION, the action's `version` input. `installed` means use the
#    dev already on PATH (this repo's CI, which builds dev from the PR).
# 2. The ref the action was used at: the last part of GITHUB_ACTION_PATH,
#    where GitHub checks the action out, or else DEV_ACTION_REF
#    (github.action_ref, which has quirks in composite actions).
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

# The ref the action was used at, e.g. v0.2.0, or empty if it's unknown. The
# checkout path comes first: it's always the ref GitHub actually fetched.
action_ref() {
  case "${GITHUB_ACTION_PATH:-}" in
    */_actions/*) echo "${GITHUB_ACTION_PATH##*/}" ;;
    *) echo "${DEV_ACTION_REF:-}" ;;
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

# Writes `prefix` (dev-<os>-<arch>-<image>-<version>-) and `key` (the prefix
# and a hash) to $GITHUB_OUTPUT. The image (e.g. ubuntu24) keeps tools built on
# one OS release from being restored onto another, where they may not run. Runs in the workspace. The hash covers dev.yaml,
# rust-toolchain.toml if there is one, and the tracked files matching each
# line of INPUT_CACHE_KEY_FILES. Language manifests are left out on purpose:
# they don't change what `dev up` installs.
cache_key() {
  [ -f dev.yaml ] || die "no dev.yaml at $(pwd); the action runs \`dev up\` from the root of the checkout"

  list=$(mktemp)
  lines=$(mktemp)
  trap 'rm -f "$list" "$list.sorted" "$lines"' EXIT
  echo dev.yaml >>"$list"
  if [ -f rust-toolchain.toml ]; then
    echo rust-toolchain.toml >>"$list"
  fi

  printf '%s\n' "${INPUT_CACHE_KEY_FILES:-}" | while IFS= read -r pattern; do
    pattern=$(printf '%s' "$pattern" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')
    [ -n "$pattern" ] || continue
    # `:(glob)` makes `**/go.sum` match at any depth, like hashFiles.
    # Unquoted, so names like café.lock come out as they are.
    matches=$(git -c core.quotePath=false ls-files -- ":(glob)$pattern") ||
      die "couldn't list the files matching cache-key-files pattern '$pattern'; is $(pwd) a git checkout?"
    [ -n "$matches" ] || die "cache-key-files pattern '$pattern' matched no tracked files"
    printf '%s\n' "$matches" >>"$list"
  done

  # A line per file, path included, so a rename changes the key too. Not in
  # a pipeline, so a file that can't be read stops the script.
  sort -u "$list" >"$list.sorted"
  while IFS= read -r file; do
    sum=$(sha256 <"$file") || die "couldn't read $file to build the cache key"
    printf '%s  %s\n' "$sum" "$file" >>"$lines"
  done <"$list.sorted"
  hash=$(sha256 <"$lines")

  os=$(printf '%s' "${DEV_ACTION_OS:?}" | tr '[:upper:]' '[:lower:]')
  arch=$(printf '%s' "${DEV_ACTION_ARCH:?}" | tr '[:upper:]' '[:lower:]')
  image=${ImageOS:-}
  if [ -z "$image" ]; then
    image=$(os_image)
  fi
  prefix="dev-$os-$arch-$image-${DEV_VERSION:?}-"
  output prefix "$prefix"
  output key "$prefix$hash"
}

# The OS and its major version, e.g. ubuntu24 or macos15, for runners that
# don't set ImageOS (self-hosted ones).
os_image() {
  case "$(uname -s)" in
    Darwin)
      v=$(sw_vers -productVersion) || die "couldn't read the macOS version; set ImageOS"
      echo "macos${v%%.*}"
      ;;
    Linux)
      [ -f /etc/os-release ] || die "can't tell which Linux this is without /etc/os-release; set ImageOS"
      # shellcheck source=/dev/null
      id=$(. /etc/os-release && echo "${ID:-}${VERSION_ID:-}")
      [ -n "$id" ] || die "/etc/os-release has no ID or VERSION_ID; set ImageOS"
      echo "${id%%.*}"
      ;;
    *) die "can't tell which OS image this is; set ImageOS" ;;
  esac
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

# Adds the dirs `dev up` set up to $GITHUB_PATH, in the order `dev
# shell-init` would put them on PATH: mise's shims, then rustup's and
# zerobrew's dirs, then ~/.local/bin (dev itself). Any other vars those
# setups set (e.g. zerobrew's PKG_CONFIG_PATH) go to $GITHUB_ENV.
#
# rustup's and zerobrew's are read from the setup dev wrote for each, so they
# can't drift from what a shell gets. mise's setup is `mise activate`, which
# only changes PATH from a prompt hook, so its shims dir is used instead.
path() {
  shell_dir="${XDG_DATA_HOME:-$HOME/.local/share}/dev/shell"
  shims="${MISE_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/mise}/shims"

  # Plain assignments, not part of a pipeline, so a failure stops the script.
  rustup=$(added_by "$shell_dir/rustup")
  zerobrew=$(added_by "$shell_dir/zerobrew")
  if [ ! -d "$shims" ]; then
    shims=
  fi

  # Highest priority first, one per line, blanks and duplicates dropped.
  dirs=$(printf '%s\n' "$shims" "$rustup" "$zerobrew" "$HOME/.local/bin" | awk 'length && !seen[$0]++')

  add_to_path "$dirs"
}

# Adds the dirs in $1 (one per line, highest priority first) to $GITHUB_PATH.
# GitHub puts each line in front of the ones before it, so they're written
# lowest priority first.
add_to_path() {
  [ -n "$1" ] || return 0
  printf '%s\n' "$1" | awk '{ lines[NR] = $0 } END { for (i = NR; i > 0; i--) print lines[i] }' >>"$GITHUB_PATH"
}

# Prints the dirs in PATH-style list $1 that aren't in $2, in order, one per
# line.
new_dirs() {
  printf '%s\n' "$1" | tr ':' '\n' | while IFS= read -r dir; do
    case ":$2:" in
      *":$dir:"*) ;;
      *) [ -z "$dir" ] || echo "$dir" ;;
    esac
  done
}

# Prints the dirs that sourcing the shell setup in $1 puts on PATH, highest
# priority first, one per line, and adds any other vars it sets to
# $GITHUB_ENV. Nothing if $1 doesn't exist.
added_by() {
  [ -f "$1" ] || return 0
  need jq "to read the shell setup in $1"
  base=/usr/bin:/bin
  # A separate shell, so the setup can't change anything in this one.
  setup=$(PATH=$base DEV_ACTION_JQ=$(command -v jq) /bin/sh "$0" load-setup "$1") ||
    die "couldn't load the shell setup in $1"

  printf '%s' "$setup" | jq -r --arg d "$(delimiter)" '
    .vars | to_entries[]
    | if (.value | contains($d)) then error("the value of \(.key) contains \($d); run the action again") else . end
    | "\(.key)<<\($d)\n\(.value)\n\($d)"' >>"$GITHUB_ENV" ||
    die "couldn't export the vars set by $1"
  new_dirs "$(printf '%s' "$setup" | jq -r .path)" "$base"
}

# Sources the shell setup in $1 and prints, as JSON, the resulting PATH and
# every other var it set or changed. Runs in its own shell (see `added_by`),
# with jq at DEV_ACTION_JQ since the setup may change PATH.
load_setup() {
  jq=${DEV_ACTION_JQ:?}
  before=$("$jq" -n env)
  # shellcheck source=/dev/null
  . "$1" >/dev/null
  # shellcheck disable=SC2016 # $before is a jq variable, not a shell one.
  "$jq" -n --argjson before "$before" '{
    path: env.PATH,
    vars: env | to_entries | map(select(.key != "PATH" and $before[.key] != .value)) | sort_by(.key) | from_entries
  }'
}

# A heredoc delimiter for $GITHUB_ENV that no value will contain.
delimiter() {
  echo "DEV_EOF_$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
}

# Fails unless $1 is on PATH, saying it's needed $2.
need() {
  command -v "$1" >/dev/null 2>&1 || die "\`$1\` is required $2, but isn't on PATH"
}

# Exports the env that dev.yaml sets, as `dev` evaluates it, to later steps:
# vars through $GITHUB_ENV, and new PATH entries through $GITHUB_PATH (never
# PATH itself, which would replace the runner's). dev's own state var isn't
# exported, so `dev <cmd>` in a later step evaluates dev.yaml again and a
# step's own overrides still reach `${VAR:-default}`.
export_env() {
  need dev "to export dev.yaml's env"
  need jq "to export dev.yaml's env"
  cd "${GITHUB_WORKSPACE:?}"
  PWD=$(pwd)
  export PWD DEV_AUTO_UPDATE=0

  base=$PATH
  script=$(dev hook-env) || die "\`dev hook-env\` failed; see its error above"
  eval "$script"
  # Nothing to export when dev.yaml has no env.
  [ -n "${__DEV_ENV:-}" ] || return 0

  delim=$(delimiter)
  names=$(printf '%s' "$__DEV_ENV" | jq -r '.baseline | keys[]') ||
    die "couldn't read the vars dev set from its state; this is a bug in dev"
  for name in $names; do
    case "$name" in
      PATH | __DEV_ENV) continue ;;
    esac
    # Unset by dev.yaml: nothing to export.
    printenv "$name" >/dev/null || continue
    # printenv adds a newline, and $(...) strips all trailing ones, so mark
    # the end to keep any the value has.
    value=$(
      printenv "$name"
      echo .
    )
    value=${value%?.}
    case "$value" in
      *"$delim"*) die "the value of $name contains $delim; run the action again" ;;
    esac
    printf '%s<<%s\n%s\n%s\n' "$name" "$delim" "$value" "$delim" >>"$GITHUB_ENV"
  done

  add_to_path "$(new_dirs "$PATH" "$base")"
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

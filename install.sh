#!/bin/sh
# shellcheck shell=sh
#
# Installs dev from its GitHub releases:
#
#   curl -fsSL https://raw.githubusercontent.com/pseudomuto/dev/main/install.sh | sh
#
# Settings, all optional:
#
#   DEV_VERSION       The version to install, e.g. 0.1.0. Defaults to the
#                     latest release.
#   DEV_INSTALL_DIR   Where to put `dev`. Defaults to ~/.local/bin.
#   DEV_DOWNLOAD_URL  Where releases are downloaded from. Defaults to
#                     https://github.com/pseudomuto/dev/releases. Mostly for
#                     tests and mirrors.
#
# The download is checked against the release's SHA256SUMS before anything is
# installed. Nothing outside DEV_INSTALL_DIR is changed: the shell setup line
# is printed for you to add.

set -eu

DOWNLOAD_URL=${DEV_DOWNLOAD_URL:-https://github.com/pseudomuto/dev/releases}

# Everything runs from here, so a download cut short by `curl | sh` runs
# nothing at all.
main() {
  for cmd in curl tar uname mktemp; do
    command -v "$cmd" >/dev/null 2>&1 || die "\`$cmd\` is required to install dev"
  done

  target=$(detect_target)
  version=${DEV_VERSION:-$(latest_version)}
  version=${version#v}
  dir=${DEV_INSTALL_DIR:-$HOME/.local/bin}

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT

  asset="dev-v$version-$target.tar.gz"
  say "Downloading dev $version ($target)"
  download "$DOWNLOAD_URL/download/v$version/$asset" "$tmp/$asset"
  download "$DOWNLOAD_URL/download/v$version/SHA256SUMS" "$tmp/SHA256SUMS"
  verify "$tmp" "$asset"

  tar -xzf "$tmp/$asset" -C "$tmp"
  [ -f "$tmp/dev" ] || die "$asset has no \`dev\` in it"

  # Copy, then rename into place, so a `dev` that's running isn't changed
  # under it, and a failed copy doesn't leave half a binary behind.
  mkdir -p "$dir"
  cp "$tmp/dev" "$dir/.dev.$$"
  chmod 755 "$dir/.dev.$$"
  mv -f "$dir/.dev.$$" "$dir/dev"

  say "Installed dev $version to $dir/dev"
  next_steps "$dir"
}

# The release target for this machine, e.g. aarch64-apple-darwin.
detect_target() {
  os=$(uname -s)
  arch=$(uname -m)

  case "$os" in
    Darwin)
      # An x86_64 shell on Apple Silicon runs under Rosetta. The native build
      # is the better one.
      if [ "$arch" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || :)" = 1 ]; then
        arch=arm64
      fi
      case "$arch" in
        arm64 | aarch64) echo aarch64-apple-darwin ;;
        x86_64) echo x86_64-apple-darwin ;;
        *) die "unsupported macOS architecture: $arch" ;;
      esac
      ;;
    Linux)
      case "$arch" in
        # Statically linked, so it runs on any distro, Alpine included.
        x86_64 | amd64) echo x86_64-unknown-linux-musl ;;
        aarch64 | arm64) echo aarch64-unknown-linux-gnu ;;
        *) die "unsupported Linux architecture: $arch" ;;
      esac
      ;;
    *) die "unsupported OS: $os (dev runs on macOS and Linux)" ;;
  esac
}

# The latest release's version, e.g. v0.1.0, from where /latest redirects.
latest_version() {
  url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$DOWNLOAD_URL/latest") ||
    die "couldn't look up the latest release at $DOWNLOAD_URL/latest"
  tag=${url##*/}
  case "$tag" in
    v[0-9]*) echo "$tag" ;;
    *) die "couldn't find the latest release at $DOWNLOAD_URL (got $url)" ;;
  esac
}

# Downloads $1 to $2.
download() {
  curl -fsSL --retry 3 -o "$2" "$1" || die "failed to download $1"
}

# Checks $2 in dir $1 against the SHA256SUMS next to it.
verify() {
  want=$(awk -v f="$2" '$2 == f || $2 == "*" f { print $1 }' "$1/SHA256SUMS")
  [ -n "$want" ] || die "SHA256SUMS has no entry for $2"

  if command -v sha256sum >/dev/null 2>&1; then
    got=$(sha256sum "$1/$2" | awk '{ print $1 }')
  elif command -v shasum >/dev/null 2>&1; then
    got=$(shasum -a 256 "$1/$2" | awk '{ print $1 }')
  else
    die "\`sha256sum\` or \`shasum\` is required to check the download"
  fi

  [ "$got" = "$want" ] || die "checksum mismatch for $2: expected $want, got $got"
}

# Prints the rc file lines to add, with a PATH line if $1 isn't on it.
next_steps() {
  say ""
  say "To finish, add this to your ~/.zshrc or ~/.bashrc and open a new shell:"
  say ""
  case ":$PATH:" in
    *":$1:"*) ;;
    *) say "  export PATH=\"$1:\$PATH\"" ;;
  esac
  say "  eval \"\$(dev shell-init)\""
}

say() {
  printf '%s\n' "$1"
}

die() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

main "$@"

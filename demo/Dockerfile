# Records demo/demo.gif with VHS (https://github.com/charmbracelet/vhs), in a
# throwaway container so `dev up` installs nothing on the host. Run it with
# `mise run demo`.

FROM rust:1.99.0-slim-trixie AS build
WORKDIR /src
# rust-toolchain.toml is left out on purpose: its target list would make
# rustup download targets this build doesn't need.
COPY Cargo.toml Cargo.lock ./
COPY src ./src
# Built into the binary for `dev shell-init` and `dev self-update`.
COPY share ./share
COPY install.sh ./
RUN cargo build --release --locked

FROM ghcr.io/charmbracelet/vhs:v0.12.1
# The zerobrew and mise installers need curl and git.
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl git \
  && rm -rf /var/lib/apt/lists/*

ENV LANG=C.UTF-8
# It's a release build, so the prompt would otherwise replace it with the
# latest published release: not the build being shown.
ENV DEV_AUTO_UPDATE=0

COPY --from=build /src/target/release/dev /usr/local/bin/dev

# The zerobrew installer refuses to run as root.
RUN useradd --create-home --shell /bin/bash demo \
  && mkdir -p /home/demo/demo /vhs \
  && chown -R demo:demo /home/demo /vhs
USER demo
COPY --chown=demo:demo demo/dev.yaml /home/demo/demo/dev.yaml
COPY --chown=demo:demo demo/demo.tape /home/demo/demo.tape

WORKDIR /home/demo
# The GIF is written to /vhs, which `mise run demo` mounts from demo/.
ENTRYPOINT ["vhs", "--output", "/vhs/demo.gif", "/home/demo/demo.tape"]

# A throwaway Linux box for trying `dev up` for real, without touching the
# host. Run it with `mise run test:docker`.

FROM rust:1.99.0-slim-trixie AS build
WORKDIR /src
# rust-toolchain.toml is left out on purpose: its target list would make
# rustup download macOS/musl targets this build doesn't need.
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:trixie-slim
# The zerobrew installer needs bash, curl, and git. zsh is there to check that
# share/shell works in it too.
RUN apt-get update \
  && apt-get install -y --no-install-recommends bash ca-certificates curl git zsh \
  && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/dev /usr/local/bin/dev
COPY share/shell /usr/local/share/dev/shell

# The zerobrew installer refuses to run as root.
RUN useradd --create-home --shell /bin/bash tester \
  && mkdir /app \
  && chown tester:tester /app
USER tester
# Same setup a dev user would add to their rc file. It makes `dev` a shell
# function, so packages `dev up` installs work right away.
RUN echo '. /usr/local/share/dev/shell' >> /home/tester/.bashrc \
  && echo '. /usr/local/share/dev/shell' >> /home/tester/.zshrc

WORKDIR /app
COPY --chown=tester:tester testdata/dev.yaml ./dev.yaml

CMD ["bash"]

# Linux smoke-test image for pgscope.
#
# Builds the app for Linux and runs it under Xvfb so the non-macOS window path
# (custom titlebar + drawn traffic lights, decorations off) can actually be
# exercised on a machine that only has macOS. See tasks.md E10.6.
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential curl ca-certificates file pkg-config \
      libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev \
      libayatana-appindicator3-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev \
      libssl-dev \
      xvfb x11-utils imagemagick \
      postgresql-client \
    && rm -rf /var/lib/apt/lists/*

# Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain stable --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"

# Node + pnpm
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y nodejs \
    && npm install -g pnpm@10 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY dev/linux-smoke/run.sh /usr/local/bin/run-smoke
RUN chmod +x /usr/local/bin/run-smoke

CMD ["/usr/local/bin/run-smoke"]

FROM rust:1.85-slim AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY kernel ./kernel
COPY veil ./veil
COPY consensus ./consensus
COPY store ./store
COPY network ./network
COPY rpc ./rpc
COPY sdk ./sdk
COPY prisms ./prisms
COPY evm ./evm
COPY node ./node
RUN cargo build --release --bin veilux

FROM debian:bookworm-slim AS runtime

RUN useradd --create-home --uid 10001 veilux \
    && apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/veilux /usr/local/bin/veilux

USER veilux
WORKDIR /home/veilux

ENV VEILUX_LOG=info

ENTRYPOINT ["veilux"]
CMD ["info"]

FROM rust:1-bookworm AS builder

WORKDIR /src
COPY Cargo.toml Cargo.lock README.md LICENSE NOTICE ./
COPY crates/ crates/
COPY conformance/ conformance/

RUN cargo build --release --bins

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 10001 -s /bin/sh saif \
    && mkdir -p /data/.saif \
    && chown -R saif:saif /data

COPY --from=builder /src/target/release/saif /usr/local/bin/saif
COPY --from=builder /src/target/release/saif-acp /usr/local/bin/saif-acp

USER saif
ENV SAIF_HOME=/data/.saif
EXPOSE 8788 8642

ENTRYPOINT ["saif"]
CMD ["status"]

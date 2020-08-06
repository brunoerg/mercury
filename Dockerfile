FROM rustlang/rust:nightly-stretch AS builder

ARG MERC_MS_TEST_SLOT
ARG MERC_MS_TEST_TOKEN
ARG MERC_DB_USER_W
ARG MERC_DB_PASS_W
ARG MERC_DB_HOST_W
ARG MERC_DB_PORT_W
ARG MERC_DB_DATABASE_W
ARG MERC_DB_USER_R
ARG MERC_DB_PASS_R
ARG MERC_DB_HOST_R
ARG MERC_DB_PORT_R
ARG MERC_DB_DATABASE_R

ENV MERC_MS_TEST_SLOT=$MERC_MS_TEST_SLOT
ENV MERC_MS_TEST_TOKEN=$MERC_MS_TEST_TOKEN
ENV MERC_DB_USER_W=$MERC_DB_USER_W
ENV MERC_DB_PASS_W=$MERC_DB_PASS_W
ENV MERC_DB_HOST_W=$MERC_DB_HOST_W
ENV MERC_DB_PORT_W=$MERC_DB_PORT_W
ENV MERC_DB_DATABASE_W=$MERC_DB_DATABASE_W
ENV MERC_DB_USER_R=$MERC_DB_USER_R
ENV MERC_DB_PASS_R=$MERC_DB_PASS_R
ENV MERC_DB_HOST_R=$MERC_DB_HOST_R
ENV MERC_DB_PORT_R=$MERC_DB_PORT_R
ENV MERC_DB_DATABASE_R=$MERC_DB_DATABASE_R

COPY . /mercury
WORKDIR /mercury

RUN set -ex \
    && apt update \
    && apt install -y \
        lsb-core \
        software-properties-common \
        apt-transport-https \
        ca-certificates \
    && bash -c "$(wget -O - https://apt.llvm.org/llvm.sh)" \
    && rm -rf /var/lib/apt/lists/*

RUN set -ex \
    && cd server \
    && cargo test -j 4 -- --test-threads=4 \
    && cargo build --release

ENV MERC_MS_TEST_SLOT=
ENV MERC_MS_TEST_TOKEN=
ENV MERC_DB_USER_W=
ENV MERC_DB_PASS_W=
ENV MERC_DB_HOST_W=
ENV MERC_DB_PORT_W=
ENV MERC_DB_DATABASE_W=
ENV MERC_DB_USER_R=
ENV MERC_DB_PASS_R=
ENV MERC_DB_HOST_R=
ENV MERC_DB_PORT_R=
ENV MERC_DB_DATABASE_R=

FROM debian:buster

COPY --from=builder /mercury/target/release/server_exec /usr/local/bin/mercury
COPY ./docker-entrypoint.sh /docker-entrypoint.sh

RUN set -ex \
    && apt update \
    && apt install -y libssl-dev \
    && rm -rf /var/lib/apt/lists/*

ENTRYPOINT ["/docker-entrypoint.sh"]

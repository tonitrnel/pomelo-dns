FROM rust:1.75-alpine3.18 as builder

WORKDIR /app

ARG PKG_VER
ARG COMMIT_ID
ARG BUILD_DATE
ARG DOCKER_VERSION
ARG RUSTC_VERSION

ENV PKG_VER=$PKG_VER
ENV COMMIT_ID=$COMMIT_ID
ENV BUILD_DATE=$BUILD_DATE
ENV RUSTC_VERSION=$RUSTC_VERSION
ENV DOCKER_VERSION=$DOCKER_VERSION
ENV SYSTEM_VERSION=alpine3.18

RUN apk update && apk add --no-cache -U musl-dev

RUN rustup upgrade

COPY ./src ./src
COPY ./Cargo.toml ./
COPY ./Cargo.lock ./

RUN cargo build --release

FROM alpine:latest

WORKDIR /app
RUN mkdir "/etc/pomelo"
RUN mkdir "/var/log/pomelo"
COPY --from=builder /app/target/release/pomelo .
COPY ./pomelo.conf /etc/pomelo/pomelo.conf
COPY ./debian/etc/logrotate.d/pomelo /etc/logrotate.d/pomelo

EXPOSE 53/tcp
EXPOSE 53/udp

RUN chmod +x ./pomelo

ENTRYPOINT ["./pomelo"]
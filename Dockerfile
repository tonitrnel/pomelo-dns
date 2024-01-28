FROM rust:1.75-alpine3.18 as builder

WORKDIR /app

RUN apk update && apk add --no-cache -U musl-dev

RUN rustup upgrade

COPY ./src ./src
COPY ./Cargo.toml ./
COPY ./Cargo.lock ./

RUN cargo build --release

FROM alpine:latest

WORKDIR /app

COPY --from=builder /app/target/release/pomelo .
COPY pomelo.conf ./pomelo.conf
COPY ./Country.mmdb ./Country.mmdb

EXPOSE 53/tcp
EXPOSE 53/udp

RUN chmod +x ./pomelo

ENTRYPOINT ["./pomelo"]
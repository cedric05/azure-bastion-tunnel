FROM rust:alpine
RUN apk add openssl pkgconfig musl-dev openssl-dev
ADD . /app/
WORKDIR /app/
RUN cargo build --release
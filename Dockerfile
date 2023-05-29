FROM rust
ADD . /app/
WORKDIR /app/
RUN cargo build --release

FROM rust:1.82.0

# Install dependencies including OpenSSL
RUN apt-get update && apt-get install -y libssl-dev pkg-config protobuf-compiler

WORKDIR /usr/src/app
COPY . .
RUN cargo build --release --features postgres

CMD ["./target/release/garbagetruck-server"]
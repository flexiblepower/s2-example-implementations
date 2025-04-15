FROM rust:1.85-slim-bullseye AS chef

WORKDIR /app
RUN apt update
RUN apt install -y libssl-dev pkg-config
COPY . .
WORKDIR /app/pv-installation
RUN cargo build --release

FROM debian:bullseye-slim
RUN apt update
RUN apt install -y libssl-dev pkg-config
COPY --from=chef app/target/release/battery /usr/local/bin/
CMD ["/usr/local/bin/battery"]
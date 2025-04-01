FROM rust:1.85-slim-bullseye AS chef

WORKDIR /app
RUN apt update
RUN apt install -y libssl-dev pkg-config
COPY . .
WORKDIR /app/pv-mock
RUN cargo build --release

FROM debian:buster-slim
RUN apt update
RUN apt install -y libssl-dev pkg-config
COPY --from=chef app/target/release/pv-mock /usr/local/bin/
CMD ["/usr/local/bin/pv-mock"]
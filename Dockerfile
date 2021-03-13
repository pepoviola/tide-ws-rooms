FROM rust:1.50 AS planner
WORKDIR /app
RUN cargo install cargo-chef
COPY . .
# Compute a lock-like file for our project
RUN cargo chef prepare  --recipe-path recipe.json

FROM rust:1.50 AS cacher
WORKDIR /app
RUN cargo install cargo-chef
COPY --from=planner /app/recipe.json recipe.json
# Build our project dependencies, not our application!
RUN cargo chef cook --release --recipe-path recipe.json

FROM rust:1.50 AS builder
WORKDIR /app
# Copy over the cached dependencies
COPY --from=cacher /app/target target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
COPY . .
# Build our application, leveraging the cached deps!
RUN cargo build --release --bin tide-ws-rooms

FROM debian:buster-slim AS runtime
WORKDIR /app
RUN apt-get update -y \
    && apt-get install -y --no-install-recommends openssl libcurl4 ca-certificates\
    # Clean up
    && apt-get autoremove -y && apt-get clean -y && rm -rf /var/lib/apt/lists/*

RUN update-ca-certificates

COPY --from=builder /app/target/release/tide-ws-rooms tide-ws-rooms
# copy assets
COPY ./public ./public

EXPOSE 5000

ENTRYPOINT ["./tide-ws-rooms"]


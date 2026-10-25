# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim AS dashboard
WORKDIR /src/dashboard
COPY dashboard/package.json dashboard/package-lock.json ./
RUN npm ci
COPY dashboard/ ./
RUN npm run build

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY scenarios scenarios
RUN cargo build --release -p shadownetd

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 shadownet
COPY --from=build /src/target/release/shadownetd /usr/local/bin/shadownetd
COPY --from=dashboard /src/dashboard/out /srv/dashboard
USER shadownet
EXPOSE 7070
ENTRYPOINT ["shadownetd", "serve", "--addr", "0.0.0.0:7070", "--static", "/srv/dashboard"]

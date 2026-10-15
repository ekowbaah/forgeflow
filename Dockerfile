FROM rust:1.97 AS build
WORKDIR /app
COPY . .
RUN cargo build --release
FROM debian:bookworm-slim
COPY --from=build /app/target/release/forgeflow /usr/local/bin/forgeflow
USER 65532:65532
ENTRYPOINT ["forgeflow"]

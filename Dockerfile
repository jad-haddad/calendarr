FROM rust:1-alpine AS builder

RUN apk add --no-cache build-base cmake perl linux-headers ca-certificates

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY src ./src
RUN touch src/main.rs \
    && cargo build --release --locked \
    && strip target/release/calendarr

FROM scratch

COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /app/target/release/calendarr /calendarr

ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
USER 1001:1001
EXPOSE 8383
ENTRYPOINT ["/calendarr"]

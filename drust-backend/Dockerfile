# Build
FROM rust:1.83 as builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main(){}" > src/main.rs && cargo build --release || true
COPY . .
RUN cargo build --release

# Runtime
FROM gcr.io/distroless/cc-debian12
COPY --from=builder /app/target/release/drust-backend /app/drust-backend
# Optional: bake local rules
#COPY --from=builder /app/rules /app/rules
##USER 65532:65532
##ENV RUST_LOG=info PORT=8080 RULES_SRC=gs://drust-core-mvp/
#ENV RULES_SRC=gs://drust-core-mvp/
ENTRYPOINT ["/app/drust-backend"]

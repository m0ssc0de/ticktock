# Start from the latest Rust image
FROM alpine as builder

# Set the Current Working Directory inside the container
WORKDIR /app

RUN apk --no-cache add rustup cargo ca-certificates openssl-dev musl-dev \
    && rm -rf /var/cache/apk/* &&  update-ca-certificates
COPY . .
# Build the binary with musl target for full static link
RUN cargo build --release

# We start from a bare Debian image so we don't get any excess libs
FROM alpine

# Copy the built binary from the builder stage
COPY --from=builder /app/target/release/ticktock /usr/local/bin

# Run the binary
CMD ["/usr/local/bin/ticktock"]

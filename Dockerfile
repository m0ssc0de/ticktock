# Start from the latest Rust image
FROM alpine as builder

# Set the Current Working Directory inside the container
WORKDIR /app

RUN apk --no-cache add rustup cargo ca-certificates openssl-dev musl-dev curl \
    && rm -rf /var/cache/apk/* &&  update-ca-certificates
RUN curl -LO "https://dl.k8s.io/release/$(curl -L -s https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl" \
    && chmod +x kubectl
COPY . .
# Build the binary with musl target for full static link
RUN cargo build  --release

# We start from a bare Debian image so we don't get any excess libs
FROM alpine
RUN apk add

# Copy the built binary from the builder stage
COPY --from=builder /app/target/release/ticktock /usr/local/bin
COPY --from=builder /usr/lib/libgcc_s.so.1 /usr/lib/libgcc_s.so.1
COPY --from=builder /app/kubectl /user/local/bin/

# Run the binary
CMD ["/usr/local/bin/ticktock"]

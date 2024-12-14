SRC := $(wildcard src/*.rs)

.PHONY: release
release: target/release/pinrs

target/release/pinrs: $(SRC)
	cargo build --release

.PHONY: debug
debug: target/debug/pinrs

target/debug/pinrs: $(SRC)
	cargo build

.PHONY: run
run: target/debug/pinrs
	cargo run

.PHONY: run-logs
run-logs: target/debug/pinrs
	RUST_LOG=warn cargo run

.PHONY: ci
ci:
	cargo fmt --all -- --check
	cargo clippy -- -D warnings
	cargo test

.PHONY: clean
clean:
	cargo clean

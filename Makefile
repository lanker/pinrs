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

.PHONY: test
test:
	cargo test

.PHONY: run-logs
run-logs: target/debug/pinrs
	RUST_LOG=warn cargo run

.PHONY: lint
lint:
	cargo fmt --all -- --check
	cargo clippy -- -D clippy::pedantic -D warnings

.PHONY: ci
ci: lint test

.PHONY: install
install: target/release/pinrs pinrs.service
	install -Dm755 -t /usr/bin $<
	install -Dm644 $(word 2,$^) /usr/lib/systemd/system/$(word 2,$^)

.PHONY: uninstall
uninstall:
	rm -f /usr/bin/pinrs
	rm -f /usr/lib/systemd/system/pinrs.service

.PHONY: clean
clean:
	cargo clean

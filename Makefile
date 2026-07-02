.PHONY: help fmt check test clippy ci core-test mix-test proto-test daemon-test cli-test

help:
	@echo "Available targets:"
	@echo "  make fmt          - Format all Rust code"
	@echo "  make check        - Check the whole workspace"
	@echo "  make test         - Test the whole workspace"
	@echo "  make clippy       - Run Clippy on all workspace targets"
	@echo "  make ci           - Run fmt, check, test, and clippy"
	@echo "  make core-test    - Test fand-core"
	@echo "  make mix-test     - Test fand-core mix tests"
	@echo "  make proto-test   - Test fand-proto"
	@echo "  make daemon-test  - Test fand daemon crate"
	@echo "  make cli-test     - Test fanctl"

fmt:
	cargo fmt --all

check:
	cargo check --workspace

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets

ci: fmt check test clippy

core-test:
	cargo test -p fand-core

mix-test:
	cargo test -p fand-core mix

proto-test:
	cargo test -p fand-proto

daemon-test:
	cargo test -p fand

cli-test:
	cargo test -p fanctl

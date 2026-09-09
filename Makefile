.PHONY: check fmt lint test audit coverage mutants

# Windows: python scripts/check.py. Make is an optional Unix convenience.
check:
	python3 scripts/check.py

fmt:
	cargo fmt --all

lint:
	cargo clippy --locked --all-targets --all-features -- -D warnings

test:
	cargo test --locked --all-features

audit:
	cargo audit --deny warnings

coverage:
	cargo llvm-cov --locked --all-features --lcov --output-path target/lcov.info

mutants:
	cargo mutants --locked --no-shuffle

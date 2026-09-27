# The everyday commands. `make` alone lists them.
.PHONY: help build release-build test test-rust test-npm lint fmt coverage demo check release

help:
	@echo "make build          debug build"
	@echo "make release-build  optimized build, in target/release/demogod"
	@echo "make test           the Rust tests, then the npm package's"
	@echo "make lint           rustfmt, clippy and rustdoc, warnings denied"
	@echo "make fmt            format the Rust code"
	@echo "make coverage       an HTML coverage report in target/llvm-cov/html"
	@echo "make demo           record docs/demo.gif"
	@echo "make check          lint and test: what CI runs"
	@echo "make release VERSION=x.y.z   version, changelog, commit and tag"

build:
	cargo build

release-build:
	cargo build --release --locked

test: test-rust test-npm

test-rust:
	cargo test --locked

test-npm: build
	cd npm && npm test

lint:
	cargo fmt --check
	cargo clippy --all-targets --locked -- -D warnings
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked

fmt:
	cargo fmt

coverage:
	cargo llvm-cov --html
	@echo "target/llvm-cov/html/index.html"

demo: release-build
	target/release/demogod docs/demo/demo.tape

check: lint test

release:
	@test -n "$(VERSION)" || (echo "usage: make release VERSION=x.y.z" && exit 1)
	scripts/release.sh $(VERSION)

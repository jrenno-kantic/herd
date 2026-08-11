# Dev-loop targets for herd.
# Run `make help` to list everything.

BLUE  := \033[34m
GREEN := \033[32m
YELLOW:= \033[33m
RED   := \033[31m
RESET := \033[0m

# Where to relocate ./target/ when `make setup-target` runs.
CARGO_PKG_NAME := $(shell awk '/^\[package\]/ {in_package=1; next} /^\[/ {in_package=0} in_package && $$1=="name" {gsub(/"/, "", $$3); print $$3; exit}' Cargo.toml)
TARGET_CACHE_DIR ?= $(HOME)/.cache/cargo-targets/$(CARGO_PKG_NAME)

.PHONY: help fmt fmt-check lint lint-fix audit check build build-release run \
        run-release test coverage verify clean setup-target teardown-target \
        version release release-minor release-major

help: ## Show this help.
	@echo "Usage:"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  $(YELLOW)%-16s$(RESET) %s\n", $$1, $$2}'

#########################
# Code hygiene
#########################

fmt: ## Format Rust code with rustfmt.
	@echo "$(BLUE)Formatting Rust...$(RESET)"
	cargo fmt --all

fmt-check: ## Check Rust formatting without writing.
	@echo "$(BLUE)Checking Rust formatting...$(RESET)"
	cargo fmt --all -- --check

lint: ## Run clippy across all targets and fail on warnings.
	@echo "$(BLUE)Running clippy...$(RESET)"
	cargo clippy --all-targets --no-deps -- -D warnings

lint-fix: ## Apply clippy's auto-fixable suggestions.
	@echo "$(BLUE)Applying clippy fixes...$(RESET)"
	cargo clippy --fix --all-targets --allow-dirty --allow-staged

audit: ## Run cargo-audit against the dependency tree. Auto-installs if missing.
	@command -v cargo-audit >/dev/null || cargo install --locked cargo-audit
	@echo "$(BLUE)Running cargo audit...$(RESET)"
	cargo audit

#########################
# Build and run
#########################

check: ## Type-check the crate without producing an executable.
	@echo "$(BLUE)Checking crate...$(RESET)"
	cargo check --all-targets

build: ## Build the crate in debug mode.
	@echo "$(BLUE)Building debug binary...$(RESET)"
	cargo build

build-release: ## Build the optimized release binary.
	@echo "$(BLUE)Building release binary...$(RESET)"
	cargo build --release

# Both run targets take over the terminal (alternate screen) until you
# press q — they are for hands-on use, never for a pipeline.
run: ## Launch the TUI in debug mode.
	@echo "$(BLUE)Launching the TUI (q to quit)...$(RESET)"
	cargo run

run-release: ## Launch the optimized TUI.
	@echo "$(BLUE)Launching the release TUI (q to quit)...$(RESET)"
	cargo run --release

#########################
# Releasing
#########################

# Versioning is deliberately *not* automatic on every release build.
#
# A build script that rewrote Cargo.toml would dirty the tree on every
# `cargo build --release`, invalidate its own fingerprint and rebuild in a
# loop, and produce version numbers that mean nothing because they count
# builds rather than releases. So the number is bumped when a release is
# cut, by asking for one — and every build in between is identified by the
# commit stamp that `build.rs` bakes in, which is the part that actually
# distinguishes one binary from another.

version: ## Print the version and the commit this tree would build.
	@cargo run --quiet -- --version

release: ## Cut a patch release: verify, bump, tag, build. VERSION=x.y.z to set it outright.
	@$(MAKE) --no-print-directory do-release BUMP=patch
release-minor: ## Cut a minor release.
	@$(MAKE) --no-print-directory do-release BUMP=minor
release-major: ## Cut a major release.
	@$(MAKE) --no-print-directory do-release BUMP=major

.PHONY: do-release
do-release:
	@test -z "$$(git status --porcelain)" || { \
		echo "$(RED)Working tree is dirty. Commit or stash first —$(RESET)"; \
		echo "$(RED)a release must be reproducible from its tag.$(RESET)"; \
		exit 1; }
	@$(MAKE) --no-print-directory verify
	@current=$$(awk -F\" '/^version = /{print $$2; exit}' Cargo.toml); \
	 if [ -n "$$VERSION" ]; then \
	   next="$$VERSION"; \
	 else \
	   next=$$(printf '%s\n' "$$current" | awk -F. -v part=$(BUMP) 'BEGIN { OFS="." } \
	      part=="major" { print $$1+1, 0, 0; next } \
	      part=="minor" { print $$1, $$2+1, 0; next } \
	                    { print $$1, $$2, $$3+1 }'); \
	 fi; \
	 echo "$(BLUE)Releasing $$current -> $$next$(RESET)"; \
	 sed -i.bak "1,/^version = /s/^version = \".*\"/version = \"$$next\"/" Cargo.toml && rm -f Cargo.toml.bak; \
	 cargo check --quiet; \
	 git add Cargo.toml Cargo.lock; \
	 git commit -q -m "release: v$$next"; \
	 git tag -a "v$$next" -m "herd v$$next"; \
	 cargo build --release; \
	 echo "$(GREEN)v$$next tagged and built.$(RESET)"; \
	 echo "Undo with: git tag -d v$$next && git reset --hard HEAD~1"

#########################
# Tests and verification
#########################

test: ## Run the Rust test suite.
	@echo "$(BLUE)Running Rust tests...$(RESET)"
	cargo test

coverage: ## Report test coverage. Auto-installs cargo-llvm-cov if missing.
	@command -v cargo-llvm-cov >/dev/null || cargo install --locked cargo-llvm-cov
	@echo "$(BLUE)Measuring coverage...$(RESET)"
	cargo llvm-cov

verify: ## Full local verification: check + clippy + fmt-check + tests + release build.
	@echo "$(BLUE)[1/5] Crate check ...$(RESET)"
	cargo check --all-targets
	@echo ""
	@echo "$(BLUE)[2/5] Clippy ...$(RESET)"
	cargo clippy --all-targets --no-deps -- -D warnings
	@echo ""
	@echo "$(BLUE)[3/5] Format check ...$(RESET)"
	cargo fmt --all -- --check
	@echo ""
	@echo "$(BLUE)[4/5] Tests ...$(RESET)"
	cargo test
	@echo ""
	@echo "$(BLUE)[5/5] Release build ...$(RESET)"
	cargo build --release
	@echo ""
	@echo "$(GREEN)✓ Verify passed.$(RESET)"

clean: ## Remove Cargo build artifacts.
	@echo "$(BLUE)Cleaning Cargo artifacts...$(RESET)"
	@if [ -L target ]; then \
		dest=$$(readlink target); \
		if [ -n "$$dest" ] && [ -d "$$dest" ]; then \
			echo "$(BLUE)Preserving target symlink → $$dest$(RESET)"; \
			find "$$dest" -mindepth 1 -maxdepth 1 -exec rm -rf {} +; \
		else \
			echo "$(YELLOW)target symlink destination does not exist: $$dest$(RESET)"; \
		fi; \
	else \
		cargo clean; \
	fi

#########################
# Build cache relocation
#########################

setup-target: ## Move ./target/ to the configured cache directory and symlink it back.
	@if [ -L target ] && [ "$$(readlink target)" = "$(TARGET_CACHE_DIR)" ]; then \
		echo "$(GREEN)✓ target → $(TARGET_CACHE_DIR) (already set up)$(RESET)"; \
	else \
		if [ -L target ]; then \
			echo "$(YELLOW)Replacing existing symlink ($$(readlink target))$(RESET)"; \
			rm target; \
		elif [ -d target ]; then \
			size=$$(du -sh target 2>/dev/null | cut -f1); \
			echo "$(BLUE)Moving existing target/ ($$size) to $(TARGET_CACHE_DIR)...$(RESET)"; \
			mkdir -p "$$(dirname "$(TARGET_CACHE_DIR)")"; \
			rm -rf "$(TARGET_CACHE_DIR)"; \
			mv target "$(TARGET_CACHE_DIR)"; \
		fi; \
		mkdir -p "$(TARGET_CACHE_DIR)"; \
		ln -s "$(TARGET_CACHE_DIR)" target; \
		echo "$(GREEN)✓ target → $(TARGET_CACHE_DIR)$(RESET)"; \
	fi

teardown-target: ## Remove the target/ symlink without deleting the cache.
	@if [ -L target ]; then \
		dest=$$(readlink target); \
		echo "$(BLUE)Removing symlink: target → $$dest$(RESET)"; \
		rm target; \
		echo "$(YELLOW)Cache at $$dest preserved. Delete manually with: rm -rf $$dest$(RESET)"; \
	else \
		echo "target is not a symlink — nothing to do"; \
	fi

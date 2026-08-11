# Dev-loop targets for ops-tui.
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
        run-release test coverage verify clean setup-target teardown-target

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

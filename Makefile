# Use bash and keep each recipe in a single shell so PATH/export persists.
SHELL := /bin/bash
.ONESHELL:

# Where installers usually drop binaries
CARGO_BIN := $(HOME)/.cargo/bin
LOCAL_BIN := $(HOME)/.local/bin

# Ensure those common install dirs are searched first
export PATH := $(CARGO_BIN):$(LOCAL_BIN):$(PATH)

.PHONY: all help ensure-rust ensure-uv pyben-develop

all: pyben-develop

help:
	@echo "Targets:"
	@echo "  make                 -> install rust & uv if needed, then build pyben (uv sync; uv run maturin develop)"
	@echo "  make ensure-rust     -> install Rust via rustup if missing"
	@echo "  make ensure-uv       -> install uv if missing"
	@echo "  make pyben-develop   -> run uv sync && uv run maturin develop in ./pyben"

ensure-rust:
	@if ! command -v rustc >/dev/null 2>&1; then \
		echo "[rust] Installing Rust (rustup) ..."; \
		curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal; \
		echo "[rust] Installed. Version: $$($(CARGO_BIN)/rustc --version)"; \
	else \
		echo "[rust] Found: $$(rustc --version)"; \
	fi

ensure-uv:
	@if ! command -v uv >/dev/null 2>&1; then \
		echo "[uv] Installing uv ..."; \
		curl -LsSf https://astral.sh/uv/install.sh | sh; \
		echo "[uv] Installed. Version: $$($(LOCAL_BIN)/uv --version 2>/dev/null || $(CARGO_BIN)/uv --version)"; \
	else \
		echo "[uv] Found: $$(uv --version)"; \
	fi

pyben-develop: ensure-rust ensure-uv
	# Make sure freshly installed binaries are picked up in this shell
	export PATH="$(CARGO_BIN):$(LOCAL_BIN):$$PATH"
	cd pyben
	uv sync
	uv run maturin develop

release: ensure-rust ensure-uv
	# Make sure freshly installed binaries are picked up in this shell
	export PATH="$(CARGO_BIN):$(LOCAL_BIN):$$PATH"
	cd pyben
	uv sync
	uv run maturin build --release

clean:
	cargo clean
	cd pyben
	rm -rf target
	rm -rf dist
	rm -rf pyben.egg-info
	rm -rf src/pyben.c
	rm -rf pyben/*abi3.so
	rm -rf pyben/pyben.*.pyd
	rm -rf pyben/__pycache__
	rm -rf .venv

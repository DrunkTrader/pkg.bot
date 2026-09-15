# Try to get the semver from 1) git 2) fallback.
VERSION := $(or $(shell git describe --tags --abbrev=0 2> /dev/null),0.0.0)
COMMIT := $(or $(shell git rev-parse --short HEAD 2> /dev/null),"unknown")
BIN := pkgbot
TARGET ?= x86_64-unknown-linux-musl

FRONTEND := site
FRONTEND_DIST := $(FRONTEND)/dist
FRONTEND_DEPS := $(FRONTEND)/node_modules/.installed
FRONTEND_SRC := $(shell find $(FRONTEND)/assets -type f)

.PHONY: build
build: build-frontend
	VERSION=$(VERSION) cargo build --release --target $(TARGET)

.PHONY: build-debug
build-debug: build-frontend
	VERSION=$(VERSION) cargo build

.PHONY: run
run: build-frontend
	cargo run

.PHONY: test
test:
	cargo test

.PHONY: clean
clean:
	cargo clean
	rm -rf $(FRONTEND_DIST)

$(FRONTEND_DEPS): $(FRONTEND)/package.json $(FRONTEND)/bun.lock
	cd $(FRONTEND) && bun install
	touch $@

$(FRONTEND_DIST)/.built: $(FRONTEND_DEPS) $(FRONTEND)/build.mjs $(FRONTEND_SRC) $(shell find $(FRONTEND)/assets -type d)
	cd $(FRONTEND) && bun run build
	touch $@

.PHONY: build-frontend
build-frontend: $(FRONTEND_DIST)/.built

.PHONY: dist
dist: build
	@echo "binary at: target/$(TARGET)/release/$(BIN)"

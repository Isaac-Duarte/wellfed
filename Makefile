.PHONY: help check check-rust check-deps check-cross check-bench-board check-virtual-feeder check-docs check-fixtures check-web check-app check-homeassistant homeassistant-wheels app-goldens app-installer-input sqlx-prepare server-dist server-image check-server-release

help:
	@echo "check: formatting, Clippy, Rust tests, cross-builds, simulator, repository checks"
	@echo "check-rust / check-deps / check-cross / check-virtual-feeder / check-docs: individual checks"
	@echo "check-bench-board: Clippy and a release build of the bench board's RP2040 firmware (part of check-cross)"
	@echo "check-fixtures: optional reference-binary/SDK evidence; fails if unavailable"
	@echo "check-web: the site and docs site: lint, format, licenses, types, unit tests, build, then Playwright (needs Node and pnpm)"
	@echo "check-app: the Android app's bindings and tokens unchanged, format, lint plugin, analysis, widget, accessibility, golden and smoke tests, and a debug APK (needs the pinned Flutter)"
	@echo "check-homeassistant: the Home Assistant integration: its bridge's Clippy with the Python module, its committed stub unchanged, Ruff, mypy strict, and its tests against the simulator with 95% coverage, on Home Assistant 2026.10 and Python 3.14 (needs uv)"
	@echo "homeassistant-wheels: the Home Assistant bridge's release wheels, cross-built for musllinux and manylinux (aarch64, x86_64, armv7 musl), and its sdist (needs uv; adds Rust targets)"
	@echo "app-goldens: the app's golden tests on Linux, in a container on any other host; UPDATE=1 writes new images back for review"
	@echo "app-installer-input: copy a review and its signed manifest in as the image the APK installs, and print its pin"
	@echo "sqlx-prepare: regenerate the server store's query metadata (.sqlx/) after changing a query or migration"
	@echo "server-dist / server-image: the server's static release binaries in dist/server/ and its container image (docker or podman)"
	@echo "check-server-release: the release check against dist/server/ and the image"
	@echo "Checks do not open serial ports, change routing, or send device commands."

check: check-rust check-deps check-cross check-virtual-feeder check-docs

# Library and binary code only: input never panics on `unwrap`, a narrowing
# `as` never wraps silently, and nothing is `pub` that no caller can reach.
# `expect` stays allowed for invariants, with the reason as its message.
PRODUCTION_LINTS = -D clippy::unwrap_used -D clippy::cast_possible_truncation \
	-D clippy::needless_pass_by_value -D unreachable_pub

# The fountain's board, which no workspace build chooses (they are the
# PLAF103's, `.cargo/config.toml`), in its own target directory so neither
# board's build replaces the other's: the board crates, the SDK bindings,
# both cores' chip routines and the chip HAL, which is what builds for it.
FOUNTAIN = WELLFED_BOARD=plwf106 CARGO_TARGET_DIR=target/plwf106
FOUNTAIN_CRATES = -p wellfed-board -p wellfed-board-plwf106 -p wellfed-amebad-sys \
	-p wellfed-hal-common -p wellfed-hal -p wellfed-fw-fountain

# After the workspace, nine lines check the bench variants' injections,
# which no workspace build enables: `stop-fail` and `feed-faults` (for
# `feed-jam` and `sensor-fault`) in the feeder drivers, and `virtual-panel`
# in the chip HAL. The last three check the fountain's board.
check-rust:
	cargo fmt --all -- --check
	cargo clippy --locked --workspace --all-targets -- -D warnings
	cargo clippy --locked --workspace --lib --bins -- -D warnings $(PRODUCTION_LINTS)
	cargo nextest run --locked --workspace --all-targets
	cargo test --locked --workspace --doc
	cargo clippy --locked -p wellfed-hal-feeder --features stop-fail --all-targets -- -D warnings
	cargo clippy --locked -p wellfed-hal-feeder --features stop-fail --lib -- -D warnings $(PRODUCTION_LINTS)
	cargo nextest run --locked -p wellfed-hal-feeder --features stop-fail --test inject
	cargo clippy --locked -p wellfed-hal-feeder --features feed-faults --all-targets -- -D warnings
	cargo clippy --locked -p wellfed-hal-feeder --features feed-faults --lib -- -D warnings $(PRODUCTION_LINTS)
	cargo nextest run --locked -p wellfed-hal-feeder --features feed-faults --test feed_faults
	cargo clippy --locked -p wellfed-hal --features virtual-panel --all-targets -- -D warnings
	cargo clippy --locked -p wellfed-hal --features virtual-panel --lib -- -D warnings $(PRODUCTION_LINTS)
	cargo nextest run --locked -p wellfed-hal --features virtual-panel --test virtual_panel
	$(FOUNTAIN) cargo clippy --locked $(FOUNTAIN_CRATES) --all-targets -- -D warnings
	$(FOUNTAIN) cargo clippy --locked $(FOUNTAIN_CRATES) --lib -- -D warnings $(PRODUCTION_LINTS)
	$(FOUNTAIN) cargo nextest run --locked $(FOUNTAIN_CRATES) --all-targets

# Licenses, advisories, duplicate versions and sources (deny.toml), then the
# out-of-workspace fuzz and bench board crates' own lockfiles under their own
# policies. Advisories need the RustSec database, which the first run fetches.
check-deps:
	cargo deny --locked check
	cargo deny --manifest-path src/fuzz/Cargo.toml --locked check --config src/fuzz/deny.toml
	cargo deny --manifest-path $(BENCH_BOARD)/Cargo.toml --locked check --config $(BENCH_BOARD)/deny.toml

# A consumer identity, because a bench one hashes the ignored build Wi-Fi file
# and this check needs no private file. The build only needs a well-formed id,
# so the fountain's KM0 archive, which has no device directory yet, takes it.
check-cross: check-bench-board
	WELLFED_BUILD_ID=$$(cargo run --quiet --locked --release -p wellfed-build -- source-id --device plaf103 --provisioning consumer) cargo build --locked -p wellfed-hal-common --release --target thumbv8m.base-none-eabi --features km0
	WELLFED_BUILD_ID=$$(cargo run --quiet --locked --release -p wellfed-build -- source-id --device plaf103 --provisioning consumer) $(FOUNTAIN) cargo build --locked -p wellfed-hal-common --release --target thumbv8m.base-none-eabi --features km0

# The bench board's firmware is its own workspace, built from its folder so
# its .cargo/config.toml (target, linker script, UF2 runner) applies. It only
# builds: nothing here touches a board or a feeder.
BENCH_BOARD = src/firmware/bench/wellfed-bench-board
check-bench-board:
	cd $(BENCH_BOARD) && cargo clippy --locked --all-targets -- -D warnings
	cd $(BENCH_BOARD) && cargo clippy --locked --bins -- -D warnings $(PRODUCTION_LINTS)
	cd $(BENCH_BOARD) && cargo build --locked --release

check-virtual-feeder:
	cargo test --locked -p wellfed-sim --test virtual_feeder

check-docs:
	cargo xtask check-repository

check-fixtures:
	cargo test --locked -p wellfed-emu -p wellfed-flash -- --ignored
	cargo test --locked -p wellfed-audit --lib -- --ignored
	cargo test --locked -p wellfed-build -- --ignored
	cargo test --locked -p wellfed-rig --lib -- --ignored

# The web frontends: the site and the docs site. Node and pnpm are needed only
# here, never by `check`, and no Rust runs.
check-web:
	pnpm -C src/web install --frozen-lockfile
	pnpm -C src/web build
	pnpm -C src/web check
	pnpm -C src/web test
	pnpm -C src/web e2e

# The Android app (src/app/). Its Dart bindings and theme tokens are
# generated from Rust and committed, so a regeneration must leave them
# unchanged. The bridge crate's own tests run in `check-rust`. The APK is
# built for arm64 phones and x86_64 emulators only.
APP = src/app
FLUTTER_VERSION = 3.47.8
APP_APK = $(APP)/build/app/outputs/flutter-apk/app-debug.apk
APP_GENERATED = $(APP)/lib/src/rust $(APP)/rust/src/frb_generated.rs $(APP)/lib/theme/tokens.g.dart
# The tests run in a zone with DST on any builder: the day-count test only proves
# anything across a DST day, and Dart's local time cannot be faked. The smoke
# tests run the real bridge against a simulator they start and stop themselves.
check-app:
	@test "$$(cd $(APP) && flutter --version --machine | sed -n 's/.*"frameworkVersion": "\(.*\)".*/\1/p')" = "$(FLUTTER_VERSION)" || \
		{ echo "check-app needs Flutter $(FLUTTER_VERSION)"; exit 1; }
	cd $(APP) && flutter_rust_bridge_codegen generate
	cargo test --locked -p wellfed-app-bridge --test tokens
	@test -z "$$(git status --porcelain -- $(APP_GENERATED))" || \
		{ git status --short -- $(APP_GENERATED); echo "generated app files are stale: commit the regenerated ones"; exit 1; }
	cd $(APP) && find lib test -name '*.dart' -not -path 'lib/src/rust/*' -exec dart format --set-exit-if-changed {} +
	cd $(APP)/tools/wellfed_lints && dart pub get && dart test
	cd $(APP) && dart analyze --fatal-infos
	cargo build --locked -p wellfed-app-bridge -p wellfed-feeder-sim
	cd $(APP) && TZ=America/Denver WELLFED_SMOKE_TARGET=$(CURDIR)/target/debug flutter test
	@if [ "$$(uname)" != Linux ]; then $(MAKE) app-goldens; fi
	cd $(APP) && flutter build apk --debug --target-platform android-arm64,android-x64
	@# Plugin-only ABI directories advertise support without a runnable app.
	@for abi in $$(unzip -Z1 "$(APP_APK)" | sed -n 's@^lib/\([^/]*\)/[^/]*\.so$$@\1@p' | sort -u); do \
		for library in libflutter.so libwellfed_app_bridge.so; do \
			unzip -Z1 "$(APP_APK)" | grep -Fxq "lib/$$abi/$$library" || \
				{ echo "the APK's $$abi directory lacks $$library"; exit 1; }; \
		done; \
	done
	@# The https verifier needs both halves on the phone: the JNI hook in the
	@# bridge and its Kotlin package, or the first https login panics.
	@for abi in arm64-v8a x86_64; do \
		unzip -p $(APP_APK) lib/$$abi/libwellfed_app_bridge.so | grep -aq Java_io_github_isaacduarte_wellfed_TlsVerifier_start || \
			{ echo "the APK's $$abi bridge lacks the TLS verifier's JNI hook"; exit 1; }; \
	done
	@unzip -p $(APP_APK) 'classes*.dex' | grep -aq 'Lorg/rustls/platformverifier/CertificateVerifier' || \
		{ echo "the APK lacks rustls-platform-verifier's Kotlin half"; exit 1; }

# The image the APK installs is a build-time input, from a frozen review and
# its signed update manifest under private/. This copies them in (they are
# ignored) and prints the pin a bench build compiles in; a release build's pin
# is committed in the bridge instead. Without either, the app builds and says
# it has no firmware to install.
APP_INSTALLER = $(APP)/assets/installer
# The goldens belong to Linux: font rasterizers differ by as much as a real
# change. On any other host they run in a Linux container, fed the app's
# sources as a tar, so the checkout gets back only the images: new ones with
# UPDATE=1, and a failed run's diffs in test/golden/failures/.
GOLDENS_IMAGE = wellfed-app-goldens:$(FLUTTER_VERSION)
GOLDENS_DIR = $(CURDIR)/$(APP)/test/golden
app-goldens:
ifeq ($(shell uname),Linux)
	cd $(APP) && flutter test test/golden $(if $(UPDATE),--update-goldens)
else
	$(CONTAINER) build -q -t $(GOLDENS_IMAGE) --build-arg FLUTTER_VERSION=$(FLUTTER_VERSION) \
		-f $(APP)/tools/goldens/Containerfile $(APP)/tools/goldens
	rm -rf $(GOLDENS_DIR)/failures && mkdir -p $(GOLDENS_DIR)/failures
	COPYFILE_DISABLE=1 tar -L -C $(APP) --exclude ./build --exclude ./.dart_tool --exclude ./test/golden/failures \
		--exclude ./android/.gradle --exclude ./android/.kotlin --exclude ./tools/wellfed_lints/.dart_tool -cf - . | \
		$(CONTAINER) run --rm -i -v wellfed-pub-cache:/root/.pub-cache \
		-v $(GOLDENS_DIR)/files:/files -v $(GOLDENS_DIR)/failures:/failures $(GOLDENS_IMAGE) \
		sh -c 'tar -x && flutter test test/golden $(if $(UPDATE),--update-goldens && cp test/golden/files/*.png /files/) \
			|| { cp -r test/golden/failures/. /failures/ 2>/dev/null; exit 1; }'
endif

app-installer-input:
	@test -n "$(REVIEW)" -a -n "$(MANIFEST)" || \
		{ echo "usage: make app-installer-input REVIEW=<review directory> MANIFEST=<signed update manifest>"; exit 1; }
	cargo run --quiet --locked --release -p wellfed-build -- installer-check --review $(REVIEW) --manifest $(MANIFEST)
	cp $(REVIEW)/review.json $(REVIEW)/build-manifest.json $(REVIEW)/image.bin $(APP_INSTALLER)/
	cp $(MANIFEST) $(APP_INSTALLER)/update.manifest
	@echo "WELLFED_INSTALLER_PIN=$$(sha256sum $(APP_INSTALLER)/review.json | cut -d' ' -f1):$$(sha256sum $(APP_INSTALLER)/update.manifest | cut -d' ' -f1):<the manifest's version>"

# The server store's queries are checked at build time against committed
# metadata, so no build needs a database. After changing a query or a
# migration, this rebuilds the metadata from a scratch database that the
# sqlite3 command line applies the migrations to.
STORE = src/server/wellfed-store
STORE_DB = $(CURDIR)/target/sqlx-prepare.db
sqlx-prepare:
	rm -rf $(STORE)/.sqlx $(STORE_DB)
	mkdir -p $(STORE)/.sqlx $(dir $(STORE_DB))
	cat $(STORE)/migrations/*.sql | sqlite3 $(STORE_DB)
	touch $(STORE)/src/lib.rs
	DATABASE_URL=sqlite://$(STORE_DB) SQLX_OFFLINE_DIR=$(CURDIR)/$(STORE)/.sqlx cargo check --locked -p wellfed-store
	rm -f $(STORE_DB)

# The server's release: static musl binaries for x86_64 and aarch64 in
# dist/server/, and the container image, both built in a container so the
# host needs only docker or podman (CONTAINER=podman). Never part of
# `check`. check-server-release runs the release check against this host's
# binary from dist/server/ and against the image; `check` already runs it
# against the debug build. docs/server/running.md#packaging.
CONTAINER ?= docker
SERVER_CONTAINERFILE = src/server/wellfed-server/packaging/Containerfile
SERVER_DIST = dist/server
SERVER_IMAGE ?= wellfed-server
server-dist:
	$(CONTAINER) build -f $(SERVER_CONTAINERFILE) --target dist --output type=local,dest=$(SERVER_DIST) .

server-image:
	$(CONTAINER) build -f $(SERVER_CONTAINERFILE) -t $(SERVER_IMAGE) .

check-server-release:
	WELLFED_RELEASE_SERVER=$(CURDIR)/$(SERVER_DIST)/wellfed-server-$$(uname -m)-unknown-linux-musl cargo nextest run --locked -p wellfed-server --test release
	WELLFED_RELEASE_IMAGE=$(SERVER_IMAGE) WELLFED_RELEASE_RUNTIME=$(CONTAINER) cargo nextest run --locked -p wellfed-server --test release --run-ignored only

# The Home Assistant integration (src/integrations/homeassistant/). Its Rust
# bridge's own tests run in `check-rust`, without the PyO3 module; this lints
# the module, builds the wheel into a venv of pinned test dependencies, and
# runs Home Assistant's tests against a simulator and the CLI they start.
# The bridge's stub is generated from the built module and committed, so a
# Rust change that alters what Python sees fails here until it is
# regenerated; mypy checks the integration against it. Every wheel ships the
# committed stub, which maturin finds beside the bridge's Cargo.toml, so a
# cross-built wheel needs no introspection of a foreign binary.
HA = src/integrations/homeassistant
HA_VENV = target/homeassistant-venv
HA_STUB = $(HA)/bridge/wellfed_homeassistant.pyi
HA_STUBS = target/homeassistant-stubs
# This host's wheel alone, so the install never picks up a cross-built one.
HA_WHEEL = target/homeassistant-wheels/check
check-homeassistant:
	cargo clippy --locked -p wellfed-homeassistant --features python --all-targets -- -D warnings
	cargo clippy --locked -p wellfed-homeassistant --features python --lib -- -D warnings $(PRODUCTION_LINTS)
	$(HA_VENV)/bin/python -c 'import sys; sys.exit(sys.version_info[:2] != (3, 14))' 2>/dev/null || uv venv --clear --python 3.14 $(HA_VENV)
	VIRTUAL_ENV=$(CURDIR)/$(HA_VENV) uv pip install -r $(HA)/tests/requirements.txt
	$(HA_VENV)/bin/ruff check $(HA)
	$(HA_VENV)/bin/ruff format --check $(HA)
	rm -rf $(HA_STUBS)
	$(HA_VENV)/bin/maturin generate-stubs --locked -q -i $(HA_VENV)/bin/python -m $(HA)/bridge/Cargo.toml --out $(HA_STUBS)
	@diff -u $(HA_STUB) $(HA_STUBS)/wellfed_homeassistant/__init__.pyi || \
		{ echo "the bridge's stub is stale: copy the generated one over $(HA_STUB)"; exit 1; }
	cargo build --locked -p wellfed -p wellfed-feeder-sim
	rm -rf $(HA_WHEEL)
	$(HA_VENV)/bin/maturin build --locked --release -i $(HA_VENV)/bin/python -m $(HA)/bridge/Cargo.toml -o $(HA_WHEEL)
	VIRTUAL_ENV=$(CURDIR)/$(HA_VENV) uv pip install --reinstall $(HA_WHEEL)/*.whl
	cd $(HA) && $(CURDIR)/$(HA_VENV)/bin/mypy
	cd $(HA) && WELLFED_BIN_DIR=$(CURDIR)/target/debug $(CURDIR)/$(HA_VENV)/bin/python -m pytest -q -p no:cacheprovider \
		--cov=custom_components/wellfed --cov-report=term-missing --cov-fail-under=95

# The wheels a release ships, cross-built with maturin and zig: musllinux for
# Home Assistant's own images (Alpine), manylinux for other installs. The
# Rust targets are added to the pinned toolchain; nothing is published.
HA_TARGETS = aarch64-unknown-linux-musl x86_64-unknown-linux-musl armv7-unknown-linux-musleabihf \
	aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu
HA_RELEASE = target/homeassistant-wheels/release
ZIG = ziglang==0.15.1
homeassistant-wheels:
	$(HA_VENV)/bin/python -c 'import sys; sys.exit(sys.version_info[:2] != (3, 14))' 2>/dev/null || uv venv --clear --python 3.14 $(HA_VENV)
	VIRTUAL_ENV=$(CURDIR)/$(HA_VENV) uv pip install -r $(HA)/tests/requirements.txt $(ZIG)
	rustup target add $(HA_TARGETS)
	rm -rf $(HA_RELEASE)
	for target in $(HA_TARGETS); do \
		PATH=$(CURDIR)/$(HA_VENV)/bin:$$PATH $(HA_VENV)/bin/maturin build --locked --release --zig \
			--target $$target -m $(HA)/bridge/Cargo.toml -o $(HA_RELEASE) || exit 1; \
	done
	$(HA_VENV)/bin/maturin sdist -m $(HA)/bridge/Cargo.toml -o $(HA_RELEASE)

.PHONY: all fmt check test test-libs test-apps prune-target

CARGO_DIRS := $(shell find . -name Cargo.toml -not -path '*/target/*' -exec dirname {} \; | sort)
LIB_DIRS   := $(filter-out ./test_apps/%,$(CARGO_DIRS))
APP_DIRS   := $(filter     ./test_apps/%,$(CARGO_DIRS))

define run-fmt-in-crates
	@set -e; \
	for dir in $(CARGO_DIRS); do \
		echo "== $$dir =="; \
		if grep -q '^\[workspace\]' "$$dir/Cargo.toml"; then \
			cargo fmt --all --manifest-path "$$dir/Cargo.toml"; \
		else \
			cargo fmt --manifest-path "$$dir/Cargo.toml"; \
		fi; \
	done
endef

# $(1) = space-separated crate dirs, $(2) = cargo subcommand (+ flags)
define run-in-dirs
	@set -e; \
	for dir in $(1); do \
		echo "== $$dir =="; \
		cargo $(2) --manifest-path "$$dir/Cargo.toml"; \
	done
endef

fmt:
	$(call run-fmt-in-crates)

check:
	$(call run-in-dirs,$(CARGO_DIRS),check)

# Run every crate, including the DB-backed test apps (needs Postgres).
test:
	$(call run-in-dirs,$(CARGO_DIRS),test --all-features)

# Library crates only (kernel + libs); no database required.
test-libs:
	$(call run-in-dirs,$(LIB_DIRS),test --all-features)

# Test apps only (todo, twitter); needs Postgres running.
test-apps:
	$(call run-in-dirs,$(APP_DIRS),test --all-features)

CI_DIRS := libs kernel
NEW_LIBS := mulac_inbox_api mulac_commanding_api mulac_eventing_api mulac_outbox_api mulac_poem mulac_axum
LIBS_MANIFEST := --manifest-path libs/Cargo.toml
# Opt-in adapter features; feature `<name>` enables the optional `mulac_<name>_api` crate.
ADAPTER_FEATURES := commanding eventing outbox
ADAPTERS := mulac_poem mulac_axum

# Format, lint and test libs and kernel; test_apps need running infrastructure.
# Existing crates still have clippy warnings, so only new crates deny them.
ci:
	@set -e; \
	for dir in $(CI_DIRS); do \
		echo "== $$dir =="; \
		cargo fmt --all --check --manifest-path "$$dir/Cargo.toml"; \
		cargo clippy --workspace --all-targets --all-features --manifest-path "$$dir/Cargo.toml"; \
		cargo test --workspace --all-features --manifest-path "$$dir/Cargo.toml"; \
	done; \
	for crate in $(NEW_LIBS); do \
		echo "== $$crate =="; \
		cargo clippy --no-deps -p $$crate --all-targets --all-features $(LIBS_MANIFEST) -- -D warnings; \
		cargo clippy --no-deps -p $$crate --all-targets --no-default-features $(LIBS_MANIFEST) -- -D warnings; \
	done; \
	echo "== dependency boundaries =="; \
	if cargo tree -p mulac_inbox_api --no-default-features --features tokio -e normal $(LIBS_MANIFEST) | grep -q diesel; then \
		echo "mulac_inbox_api without diesel must not depend on Diesel"; exit 1; fi; \
	if cargo tree -p mulac_poem --all-features -e normal $(LIBS_MANIFEST) | grep -qE ' (axum|utoipa) '; then \
		echo "mulac_poem must not depend on Axum"; exit 1; fi; \
	if cargo tree -p mulac_axum --all-features -e normal $(LIBS_MANIFEST) | grep -qE ' poem'; then \
		echo "mulac_axum must not depend on Poem"; exit 1; fi; \
	if cargo tree -p mulac_axum -e normal $(LIBS_MANIFEST) | grep -q ' utoipa '; then \
		echo "mulac_axum must not require utoipa"; exit 1; fi; \
	if cargo tree -p mulac_outbox_api --all-features -e normal $(LIBS_MANIFEST) | grep -q ' lapin '; then \
		echo "mulac_outbox_api must not depend on lapin"; exit 1; fi; \
	for adapter in $(ADAPTERS); do \
		if cargo tree -p $$adapter --all-features -e normal $(LIBS_MANIFEST) | grep -q ' lapin '; then \
			echo "$$adapter must not depend on lapin"; exit 1; fi; \
	done; \
	for feature in $(ADAPTER_FEATURES); do \
		api=mulac_$${feature}_api; \
		echo "== adapter feature $$feature =="; \
		if cargo tree -p $$api --no-default-features --features tokio -e normal $(LIBS_MANIFEST) | grep -q diesel; then \
			echo "$$api must not depend on Diesel"; exit 1; fi; \
		for adapter in $(ADAPTERS); do \
			cargo clippy --no-deps -p $$adapter --all-targets --no-default-features --features $$feature $(LIBS_MANIFEST) -- -D warnings; \
			if cargo tree -p $$adapter -e normal $(LIBS_MANIFEST) | grep -q " $$api v"; then \
				echo "$$adapter must not include $$api by default"; exit 1; fi; \
			if cargo tree -p $$adapter --no-default-features --features $$feature -e normal $(LIBS_MANIFEST) | grep -q diesel; then \
				echo "$$adapter with only $$feature must not depend on Diesel"; exit 1; fi; \
		done; \
		cargo clippy --no-deps -p mulac_axum --all-targets --no-default-features --features $$feature,utoipa $(LIBS_MANIFEST) -- -D warnings; \
	done

# Static registry checkout deployed by GitHub Pages from main.
CRATES_DIR ?= $(HOME)/work/labs/crates
MARGO ?= margo
CRATES_PUSH_URL ?= https://github.com/null-labs-hq/crates.git

.PHONY: package publish pack_and_publish test-publishing

package:
	cargo package --no-verify --manifest-path libs/Cargo.toml --workspace
	cargo package --no-verify --manifest-path kernel/Cargo.toml

publish: pack_and_publish

pack_and_publish:
	python3 scripts/pack_and_publish.py --registry "$(CRATES_DIR)" --margo "$(MARGO)" --deploy --push-url "$(CRATES_PUSH_URL)"

test-publishing:
	python3 -m unittest discover -s scripts -p 'test_*.py'

# Keep only the newest KEEP builds per package in every target/ dir.
# `make prune-target KEEP=3`, or `make prune-target DRY_RUN=1` to only list what would go.
KEEP ?= 2
prune-target:
	@find . -path ./.git -prune -o -type d -regex '.*/target/[^/]*/\(deps\|\.fingerprint\|build\|incremental\)' -prune -print | \
	while read -r d; do \
		find "$$d" -mindepth 1 -maxdepth 1 -printf '%T@\t%p\n' | sort -rn | \
		awk -F'\t' -v N=$(KEEP) '{p=$$2; f=p; sub(/.*\//,"",f); b=f; sub(/\..*/,"",b); h=b; sub(/.*-/,"",h); n=b; sub(/-[^-]*$$/,"",n); if (f ~ /\.(rlib|rmeta|so|a|dylib)$$/) sub(/^lib/,"",n); k=n SUBSEP h; if (!(k in s)) s[k]=++c[n]; if (s[k]>N) print p}'; \
	done | \
	if [ -n "$(DRY_RUN)" ]; then cat; else xargs -r -d '\n' rm -rf; fi

test-apps-up:
	@$(MAKE) -C test_apps up

test-apps-down:
	@$(MAKE) -C test_apps down

test-apps-reset:
	@$(MAKE) -C test_apps reset

test-apps-test:
	@$(MAKE) -C test_apps/todo test
	@$(MAKE) -C test_apps/twitter test

test-apps-serve:
	@$(MAKE) -C test_apps/todo serve
	@$(MAKE) -C test_apps/twitter serve

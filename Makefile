.PHONY: fmt check test package publish test-apps-up test-apps-down test-apps-reset test-apps-test test-apps-serve

CARGO_DIRS := $(shell find . -name Cargo.toml -not -path '*/target/*' -exec dirname {} \; | sort)

KERNEL_VERSION := $(shell cargo pkgid --manifest-path kernel/Cargo.toml | sed 's/.*[#@]//')
LIBS_VERSION   := $(shell awk -F'"' '/^version/{print $$2; exit}' libs/Cargo.toml)

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

define run-in-crates
	@set -e; \
	for dir in $(CARGO_DIRS); do \
		echo "== $$dir =="; \
		cargo $(1) --manifest-path "$$dir/Cargo.toml"; \
	done
endef

fmt:
	$(call run-fmt-in-crates)

check:
	$(call run-in-crates,check)

test:
	$(call run-in-crates,test --all-features)

MARGO_PROJECT := /home/nulllabs/work/labs/margo

define publish-lib
	cargo package --no-verify --manifest-path libs/$(1)/Cargo.toml
	$(MAKE) -C $(MARGO_PROJECT) add CRATE=$(CURDIR)/libs/target/package/$(2)-$(LIBS_VERSION).crate
endef

package:
	@echo "== libs v$(LIBS_VERSION) =="
	cargo package --no-verify --manifest-path libs/Cargo.toml --workspace
	@echo "== kernel v$(KERNEL_VERSION) =="
	cargo package --no-verify --manifest-path kernel/Cargo.toml

publish:
	$(call publish-lib,mulac_diesel,mulac_diesel)
	$(call publish-lib,eventing,mulac-eventing)
	$(call publish-lib,commanding,mulac-commanding)
	$(call publish-lib,outbox,mulac-outbox)
	$(call publish-lib,inbox,mulac-inbox)
	cargo package --no-verify --manifest-path kernel/Cargo.toml
	$(MAKE) -C $(MARGO_PROJECT) add CRATE=$(CURDIR)/kernel/target/package/mulac-kernel-$(KERNEL_VERSION).crate

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

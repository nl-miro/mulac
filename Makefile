.PHONY: fmt check test

CARGO_DIRS := $(shell find . -name Cargo.toml -not -path '*/target/*' -exec dirname {} \; | sort)

define run-in-crates
	@set -e; \
	for dir in $(CARGO_DIRS); do \
		echo "== $$dir =="; \
		cargo $(1) --manifest-path "$$dir/Cargo.toml"; \
	done
endef

fmt:
	$(call run-in-crates,fmt)

check:
	$(call run-in-crates,check)

test:
	$(call run-in-crates,test --all-features)

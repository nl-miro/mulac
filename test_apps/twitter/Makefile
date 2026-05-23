DATABASE_URL ?= postgres://twitter:twitter@127.0.0.1:5433/twitter
BIND_ADDR ?= 127.0.0.1:33002

.PHONY: migrate test serve check

migrate:
	DATABASE_URL=$(DATABASE_URL) cargo run -- migrate

test:
	DATABASE_URL=$(DATABASE_URL) cargo test

check:
	cargo fmt --check
	cargo check

serve:
	DATABASE_URL=$(DATABASE_URL) BIND_ADDR=$(BIND_ADDR) cargo run -- serve

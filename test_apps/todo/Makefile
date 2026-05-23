DATABASE_URL ?= postgres://todo:todo@127.0.0.1:5433/todo
BIND_ADDR ?= 127.0.0.1:3000

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

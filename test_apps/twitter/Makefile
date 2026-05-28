DATABASE_URL ?= postgres://twitter:twitter@127.0.0.1:5433/twitter
BIND_ADDR ?= 127.0.0.1:33002

.PHONY: migrate test serve check

migrate:
	DATABASE_URL=$(DATABASE_URL) cargo run -- migrate

fmt:
	cargo fmt

fix:	
	cargo fix --allow-dirty

check:
	cargo fmt --check
	cargo check
	
test:
	DATABASE_URL=$(DATABASE_URL) cargo test
	#DATABASE_URL=$(DATABASE_URL) RUSTFLAGS="-A warnings" cargo test -q -- --quiet

serve:
	DATABASE_URL=$(DATABASE_URL) BIND_ADDR=$(BIND_ADDR) cargo run -- serve

wip:
	git add . && git commit -am 'wip'

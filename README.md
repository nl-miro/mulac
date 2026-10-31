# mulac

mulac is a set of architecture building blocks for reliable event-based systems (Inbox/Outbox + pluggable command and event dispatchers).

This repository contains both technical specifications and Rust crate implementations for the building blocks.

## Docs

- Fast repo TL;DR: [docs/repo-tldr.md](docs/repo-tldr.md)
- High-level component view: [docs/components.md](docs/components.md)
- Detailed spec (end-to-end flows): [docs/architecture-spec.md](docs/architecture-spec.md)
- Lifecycle states and rules: [docs/components.md](docs/components.md)
- Outbox technical specification: [docs/outbox-spec.md](docs/outbox-spec.md)

## Inbox HTTP adapters

Services expose `POST /inbox` and `POST /inbox/run` through shared crates
([ADR-009](docs/adr/009-inbox-http-adapters.md)):

- [`libs/mulac_inbox_api`](libs/mulac_inbox_api) — framework-free contract, validation and service.
- [`libs/mulac_poem`](libs/mulac_poem) — Poem/poem-openapi adapter.
- [`libs/mulac_axum`](libs/mulac_axum) — Axum adapter with optional Utoipa schemas.

## Queue HTTP adapters

`POST /commands/run`, `POST /events/run` and `POST /outbox/run` follow the
same pattern ([ADR-010](docs/adr/010-queue-http-apis.md)):
[`libs/mulac_commanding_api`](libs/mulac_commanding_api),
[`libs/mulac_eventing_api`](libs/mulac_eventing_api) and
[`libs/mulac_outbox_api`](libs/mulac_outbox_api) hold the contracts and services, and
both adapters expose them behind their opt-in `commanding`, `eventing` and
`outbox` features. The outbox answers `503` while no publisher is configured.

Run `make ci` to format-check, lint and test `libs` and `kernel`, to verify
that each adapter depends only on its own framework, and to check that each
opt-in adapter feature builds alone, stays out of the default build and pulls
in no Diesel. It also checks that `mulac_outbox_api` and the adapters never depend
on lapin.

## Optional application event revisions

`ApplicationEvent::sequence()` defaults to `None`. Applications may supply a
positive per-aggregate revision up to PostgreSQL BIGINT's maximum. The typed
command wrapper carries it through event metadata without changing framework
IDs, correlation, causation or payloads. Event reservation and subscriber
conversions preserve it. Outbox metadata has the same optional field; recorders
must copy it from the originating event. Missing legacy metadata stays missing.
This supplies provenance, not ordering or atomic command/event persistence.

## Publish crates

Run `make pack_and_publish` (or `make publish`) to package all libraries and the
kernel in dependency order, add them to `~/work/labs/crates`, commit the registry
artifacts, and push its `main` branch. GitHub Pages then deploys the registry at
<https://null-labs-hq.github.io/crates/>. This does not commit or push mulac itself.

Requires Python 3.11+, Cargo, `margo`, an initialized registry checkout on `main`,
and GitHub push access. HTTPS authentication uses `gh auth git-credential` when
`gh` is installed. Override `CRATES_DIR`, `MARGO`, or `CRATES_PUSH_URL` as needed.
Keep unrelated registry changes out of the generated index/assets and crate paths.

Packaging uses a temporary loopback sparse registry so new dependencies resolve
before Pages deploys; published manifests retain the hosted registry URL. Identical
retries are safe, and an existing version with different bytes is rejected.
Packaging retains `--no-verify`; run `make ci` before publishing a release.

For local staging without committing or pushing:

```sh
python3 scripts/pack_and_publish.py --registry "$HOME/work/labs/crates"
```

Run `make test-publishing` for publishing helper tests. A failed push leaves the
local registry commit intact; fix authentication or reconcile remote changes and
retry. No force-push is performed.

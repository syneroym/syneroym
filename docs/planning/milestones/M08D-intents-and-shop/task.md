# M08D: Intents and shop

*Reader: developers and the project owner, when they plan this sub-milestone.*

**Status: outline (2026-10-26).** This is a first outline. The full plan is written when M08C is near its end. Milestone overview and order: [M08-overview.md](../M08-overview.md).

## Goal

Offerings and wants become one intent record, with in-return terms (what an offering accepts, what a want can give). Every participant gets a shop: images, attributes, variants, sections, links, and one browse query that works on one shop and at a directory.

Both parts reshape `listing`, so they are built together, once.

**Demo:** New story for offerings and the shop. A viewer sees one person publish an offering with images and attributes, and another person browse that shop.

## Change docs

- [roym-value-exchange](../../changes/roym-value-exchange/change.md): intents, offerings and wants (`VX-1`, `VX-4`).
- [roym-shop](../../changes/roym-shop/change.md) (`SH-*`).
- [roym-intent-matching](../../changes/roym-intent-matching/change.md): the intent fields only (`IM-1`, `IM-2`).

## Slices (first cut)

| Slice | Content | Needs |
| --- | --- | --- |
| D0 | Intents module change doc (exact intent record, in-return terms). | — |
| D1 | Intent record (offer or want) in `catalog`, with in-return terms. `listing` becomes the offer kind. | D0, M08C |
| D2 | `blob-store` for Roym services. Images, thumbnails, serving by hash. | — |
| D3 | Attribute values and variants on offerings. `storefront` record, collections, featured, links, record link grammar. | D1, M08B taxonomy |
| D4 | Browse query: on the owner's node (wire-reachable, read-only) and at the directory, with attribute and price columns and facets. Thumbnails stored at the directory. | D3, M08B FTS5 |
| D5 | Hub: shop page, offering page, filter panel. `roymctl` browse. Close-out. | D4 |

## Open before planning

- Image limits, and an image crate that builds for `wasm32-wasip2` (shop change).
- Per-member storage cap at a directory or exchange for thumbnails.
- Want screens arrive in M08F. D1 builds the record and its validation only.

---
status: draft
living-docs-touched:
---
# Roym shop primitives

*Reader: developers who plan and build Milestone 8, and the project owner.*

This change gives every participant a **shop**: a browsable set of their offerings with images, attributes, variants, sections and links. A buyer can browse one shop, or filter offerings from many shops through a directory (an exchange after M08F), with the same query. Every answer is JSON with stable record links. The Hub renders it as pages. Any other client can render it in its own way.

Terms used here:

- **Participant, offering, want, intent, exchange:** see the [Value Exchange](../roym-value-exchange/change.md) change. An **offering** is today's `listing`. An **intent** is an offering or a want.
- **Directory:** today's SynOrg search service (`roym_directory`). This change is built in M08D at the directory. In M08F the directory's search half becomes the **exchange**, and everything this document says about the directory moves with it.
- **Taxonomy:** the shared list of category codes with labels per language (see the [multilingual](../roym-multilingual/change.md) change). This change adds attribute definitions to it.
- **Attribute:** a typed property of an offering, defined per category. Examples: `fabric = cotton` for clothes, `veg = true` for food, `seats = 4` for a taxi.
- **Facet:** a count of matching offerings per attribute value, returned with a browse answer so a client can show filters ("Cotton (12)").
- **Variant:** one buyable version of an offering, such as size M in blue.
- **Collection:** a named section of one shop, such as "Breakfast" or "Kids".
- **Storefront:** a signed record that describes one shop as a whole.
- **Record link:** a stable reference to a signed record, which any client can resolve.

## Problem

A listing today has a title, a summary, up to 8 free-form categories, prices, and product or service details (`crates/roym_core/src/listing.rs:177`). It has:

- no images (the Roym services do not import the `blob-store` interface today);
- no typed attributes, so a buyer cannot filter by size, material or diet;
- no variants;
- no way to group listings into sections or to link related ones;
- no shop page. A buyer sees single listings in a directory search, never one participant's whole range.

Directory search filters only by category, area, free text, `open_to` and booking mode. Price and product terms are signed but not filterable (a deferred-backlog row from M06C).

## Scope and non-goals

- In scope:
  - Images on offerings and on the storefront: stored in the owner's blob store, referenced by content hash, with a small thumbnail.
  - Attribute definitions per taxonomy category, and attribute values on offerings.
  - Variants with their own attribute values and price.
  - Collections and featured offerings, in a signed storefront record.
  - Links between offerings (own and other participants').
  - One **browse query** shape, used on a participant's own node (to browse one shop) and at a directory (to browse many). It returns results, facets and a cursor.
  - Wire-reachable, read-only shop methods on a participant's node, so another node can browse a shop with no directory.
  - Hub pages: shop page, offering page with images, filter side panel.
- Not in scope:
  - Stock counts, carts, delivery tracking. These are work steps, so they belong in [Roym extensions](../roym-extensions/change.md).
  - Video. Images only in M8.
  - A web page for visitors who have no node (for example a link opened in any browser). The JSON answer would serve it later. The delivery path (the WebRTC browser path or a public gateway) is a backlog row.
  - Ranking by reputation. See the [intent matching](../roym-intent-matching/change.md) change.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| **Images** | | |
| SH-1 | An owner adds up to a fixed number of images to an offering and to the storefront. Each image is stored in the owner's blob store. The signed record holds its content hash, media type, width, height, and alt text with a language tag. | |
| SH-2 | The owner's node makes one thumbnail per image, up to a size cap. The thumbnail's hash is in the signed record too. | |
| SH-3 | A client gets a full image from the owner's node by hash. A changed image byte fails the hash check and is not shown. | |
| SH-4 | A directory stores the thumbnails of the offerings published to it, so search results show images when the owner's node is offline. | |
| **Attributes and variants** | | |
| SH-5 | The taxonomy defines attributes per category: a code, a type (choice, number with unit, yes or no, text), allowed value codes for a choice, and labels per language. A child category has its parent's attributes too. | |
| SH-6 | An offering carries attribute values. A value of a defined attribute is checked against its type. A value of an unknown attribute is allowed, and shown as raw code and text. | |
| SH-7 | An offering can have variants. Each variant has attribute values, a price, and an optional SKU. A buyer's enquiry names one variant. | |
| **Storefront, collections, links** | | |
| SH-8 | A participant signs one storefront record: name, about text, language tag, images, collections (each with a name and an ordered list of offering ids) and featured offerings. | |
| SH-9 | An offering can link to other offerings by record link, with a relation code (`related`, `accessory`, `alternative`, `part-of`). A link to another participant's offering is verified when it is shown. | |
| SH-10 | A record link has one text form that any client can parse and resolve: the owner's DID, the record type and the record id. | |
| **Browse** | | |
| SH-11 | One browse query shape: category code (with sub-categories), attribute filters, price range in one currency, area, text, intent kind, sort, page size and cursor. | |
| SH-12 | A participant's node answers the browse query over its own offerings, through a wire-reachable read-only method. It returns only active offerings that the owner has published. | |
| SH-13 | A directory answers the same query over the offerings published to it, and returns facets: counts per attribute value and per sub-category for the current filters. | |
| SH-14 | Each result is the signed offering itself (or a projection that names the record and its hash), plus its thumbnail hash and source. The client verifies it as it does today. | |
| SH-15 | A browse answer is JSON only. The Hub renders it as a shop page and a filtered list. A test drives the same answer through `roymctl` to prove no Hub logic is needed. | |

## Design

### Images

- Roym services get the `blob-store` host interface. Images are written to the owner's blob store, which is already content-addressed and encrypted at rest.
- The record holds `{hash, media_type, width, height, bytes, thumb_hash, alt, lang}` per image. Limits (number of images, bytes per image, thumbnail size) are set in the plan. A first value: 8 images, 2 MB each, 32 KB thumbnails.
- Thumbnails are made on the owner's node at upload, in Rust (`unverified`: an image crate that builds for `wasm32-wasip2` with the needed formats). If none fits, the Hub makes the thumbnail in the browser and the node checks only size and type.
- Full images are served by a wire-reachable read method on the owner's node, by hash, for published offerings only.

### Attributes in the taxonomy

The multilingual change adds a taxonomy of category codes. This change adds attribute definitions to it, in the same data file. Values of a choice attribute are codes with labels, so filtering works across languages with no translation. Text attributes are free text with the record's language tag.

### Storefront and collections

`storefront` is a new signed record type, one current version per participant, published with the participant's offerings. Collections live in the storefront, not in each offering, so reordering a section changes one record.

### Record links

Text form: `roym:<owner-did>/<record-type>/<record-id>`. The exact grammar is set in the plan. A client resolves a link by asking the owner's node (read method), or a directory that holds the record. The link carries no host or address. Resolution uses the normal discovery path.

### Browse query

```text
kind:        offer | want
category:    taxonomy code; matches the code and its sub-categories
attributes:  [ { code, op: eq | in | range, value(s) } ]
price:       { currency, min_minor, max_minor }
area:        an Area (as today)
text:        free text
sort:        relevance | newest | price-asc | price-desc
page:        { size, cursor }
```

The same shape serves:

1. **One shop**, on the owner's node. No facets are needed for a small shop, but the same code returns them.
2. **Many shops**, at a directory. The directory's index adds attribute and price columns to today's projection.
3. **Many directories**, through the federated query library (see [Roym open foundation](../roym-open-foundation/change.md)).

The [intent matching](../roym-intent-matching/change.md) change uses this query shape as the "hard filter" step.

## Open questions

- **Price filter across currencies.** Recommendation: a price filter names one currency and skips offerings in other currencies. No conversion.
- **Directory storage per offering.** Thumbnails add bytes per offering at each directory. Set a per-member cap at the directory.
- **Who adds attributes to the taxonomy.** The same process as category codes (multilingual change, open question).

## Tasks

- [ ] Write the M08D slice plans (together with the intent record change from the Value Exchange change, because both reshape `listing`).
- [ ] `blob-store` for Roym services. Image upload, thumbnail, serve by hash.
- [ ] Attribute definitions in the taxonomy. Attribute values and variants on offerings.
- [ ] `storefront` record. Collections, featured, links. Record link grammar.
- [ ] Browse query on the owner's node (wire-reachable, read-only) and at the directory, with facets.
- [ ] Hub: shop page, offering page, filter panel. `roymctl` browse.

## Deviations

## Close-out

Before merge, update the living docs (integrated spec, requirements, architecture, terminology) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).

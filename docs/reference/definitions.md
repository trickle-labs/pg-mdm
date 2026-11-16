# Definition reference

[Documentation home](../README.md) · Reference

An entity definition is a JSON object assembled by the five SQL constructors or supplied directly to `mdm.create()`. The constructors provide readable named arguments and early checks; creation validates the complete definition, source contracts, selected role, and producer capabilities. This reference describes the current implementation. Use [Model sources](../how-to/model-sources.md) for task-oriented advice and [SQL API](sql-api.md) for exact signatures.

## Entity structure

| Property | Contract |
| --- | --- |
| `name` | Nonempty entity name; output names including `_members` and `_review` must fit PostgreSQL's 63-byte limit. |
| `sources` | Nonempty array of source declarations, with unique source names. |
| `fields` | Nonempty array of logical fields, with unique field names. |
| `matches` | Nonempty array of match rules, with unique rule names. |
| `golden_values` | Array of golden policies; each field can appear once and must be a declared field. |
| `preset` | Optional `person`, `company`, or `product`, expanded to version 1 metadata. Explicit sources, fields, rules, and policies are still required. |
| `limits` | Object of supported positive integer resource bounds; defaults are expanded into the retained definition. |
| `execution_role` | Optional role name subject to caller validation and entity binding checks. |

Definitions are canonicalized for deterministic digests. Named collections, including sources, are sorted by name, while semantically meaningful arrays are preserved where required. This has a visible consequence for golden values: source priority currently follows the canonical source-name order. Definition version and artifact version describe different concerns; changing the compiler can replace an artifact without redefining the user's matching policy. See [Publication and identity](../explanation/publication-and-identity.md).

## Source contracts

`relation` identifies an existing ordinary or partitioned, non-temporary PostgreSQL table. `source_id` is an ordered, nonempty list of columns backed by a valid, ready, immediate primary or unique index with the same key columns in the same order. Expression and partial indexes do not satisfy the contract. Key columns must all be `NOT NULL`, or the unique index must use `NULLS NOT DISTINCT`. The retained key contract includes column types, collations, relation name, and encoding; changing it is not an ordinary source update.

| Source property | Values and behavior |
| --- | --- |
| `mode` | `tracked` or `soft_delete`. |
| `fields` | Object mapping logical field names to physical source-column names. Mappings must match the declared types. |
| `row_changed_at` | Optional timestamp column used by timestamp-sensitive value selection. |
| `soft_delete_when` | Required for `soft_delete`, forbidden for `tracked`; object with `column` and `kind` equal to `is_true` or `is_not_null`. |
| `authority` | Object whose values are strings; marks configured fields with source authority metadata. |

The execution role needs schema `USAGE` and source-table `SELECT, MAINTAIN`. Row-level security is evaluated under that role; when an execution role owns a source with RLS, the source contract requires `FORCE ROW LEVEL SECURITY` rather than relying on owner bypass. The changed-key optimization applies to a narrower set of source shapes than the complete source contract. See [Operations](../how-to/operate.md#understand-where-incremental-work-still-has-a-cost) before interpreting a valid source as a promise of incremental source scanning.

## Cleaners

The current cleaners use semantic version 1. They create normalized comparison values without rewriting source data. Normalized states distinguish `value`, `absent`, `empty`, `invalid`, `unknown`, `redacted`, and `unsupported`; only `value` supplies comparison evidence. Two missing or invalid values therefore do not become an exact identity match merely because neither has usable content.

| Cleaner | Main behavior | Default maximum input length |
| --- | --- | ---: |
| `none` | Preserves a present usable value; blank text is empty. | 4096 |
| `text` | Unicode NFKC normalization, lowercasing, trimming, and whitespace collapse. | 4096 |
| `person_name` | Text normalization plus replacement of ASCII punctuation with spaces and whitespace collapse. | 4096 |
| `company_name` | Currently uses the same normalization as `person_name`; it does not expand company abbreviations. | 4096 |
| `email` | NFKC normalization, outer trimming, structural checks, and lowercasing of local and domain parts; preserves dots and plus tags. | 320 |
| `phone` | NFKC normalization, optional leading `+`, removal of supported visual separators, and 7–15 ASCII digits; does not infer a country code. | 64 |
| `tax_id` | NFKC normalization, uppercase ASCII alphanumeric content, and removal of spaces and hyphens; no country-specific check-digit validation. | 128 |
| `date` | Date normalization through the date field/cleaner path. | 64 |

`cleaner_options` accepts `max_length`, a positive integer that can lower the cleaner's maximum but cannot raise the compiled default. Unsupported options are rejected. Text cleaners require a text-compatible field; `date` requires a date field. The declared logical types accepted by the constructor are `text`, `character varying`, `character`, `date`, `timestamp`, `timestamp without time zone`, `timestamp with time zone`, `boolean`, `smallint`, `integer`, `bigint`, and `numeric`; acceptance still depends on the selected cleaner and compatible source mapping.

`display` is `full`, `masked`, or `hashed`, with `masked` as the default. Treat it as presentation metadata for explanations, not as a database privilege or a promise that golden columns are masked. The three primary public tables are protected by ordinary PostgreSQL grants.

## Match rules and evidence groups

| Property | Contract |
| --- | --- |
| `fields` | Nonempty list of declared logical fields, compatible with the comparator. |
| `comparison` | `exact`, `normalized_levenshtein`, or alias `fuzzy`. |
| `strength` | `identity`, `strong`, or `supporting`. |
| `evidence_group` | Nonempty name identifying an independent evidence lineage. Reusing the same lineage across different groups is rejected. |
| `threshold` | Required for fuzzy comparison, integer 0–10000; forbidden for exact comparison. |
| `candidate` | Candidate predicate for an owning rule. Supporting rules cannot own candidates; strong and non-exact identity rules require one. Exact identity rules can infer their exact candidate. |

Normalized Levenshtein scores use 10000 for the maximum similarity. The threshold is a deterministic cutoff for that comparator, not a calibrated probability that two records represent the same person or company. Supporting evidence can contribute context but does not create candidates or independently justify an automatic identity link. The [resolution explanation](../explanation/entity-resolution.md) covers pair evidence and conservative component admission.

## Candidate predicates

Candidate predicates operate on normalized fields and define the set of pairs a rule can examine. The engine evaluates complete candidate sets within configured limits; it does not silently keep only the most promising pairs. The following JSON shapes are accepted examples for declared fields named `email`, `name`, and `country`:

| Kind | Example | Meaning |
| --- | --- | --- |
| Exact | `{"kind":"exact","field":"email"}` | Same normalized value for one field. |
| Composite exact | `{"kind":"composite_exact","fields":["name","country"]}` | Agreement on the declared composite key. |
| Prefix | `{"kind":"prefix","field":"name","length":4}` | Shared normalized prefix of positive length. |
| Token | `{"kind":"token","field":"name","min_length":3}` | Shared eligible token with a positive minimum length. |

Use `field` for single-field predicates and `fields` for a composite predicate. A single-field predicate requires the owning rule's `fields` array to contain exactly that field. A composite predicate's `fields` must match the rule's fields in the same order; a subset is not sufficient. Duplicate rules can contribute the same candidate pair; the candidate set is deduplicated before pair evaluation. Predicate choice changes recall as well as cost: no comparator can recover a pair that every declared candidate channel excludes.

## Golden-value policies

Golden selection considers active eligible records with usable normalized values. It retains the selected source value and provenance, so the published text can preserve source spelling even though a normalized form was used for comparison. An override may supersede ordinary selection, and conflicting or inactive-anchor directives can create review work.

| Policy | Ordering and requirements |
| --- | --- |
| `prefer_source` | Requires a nonempty eligible-source list; chooses by canonical source-name priority, then latest row timestamp and deterministic source ordering. |
| `latest` | Considers usable records with a known `row_changed_at`; newest timestamp wins, followed by source priority and deterministic source ordering. |
| `first_non_null` | Chooses a usable value by source priority and deterministic source ordering; missing and invalid values are not usable candidates. |
| `most_common` | Groups canonical values; uses frequency, authoritative support, source priority, and canonical ordering as deterministic tie breaks. |

The `sources` array filters eligibility for every policy that uses it. **Its order does not define priority in the current implementation.** If your business requirement depends on a particular fallback ordering, verify that it is expressible by the current policy instead of assuming the array implements it. Published golden fields cannot subsequently be removed, renamed, reordered, or retyped through an ordinary definition change.

## Resource limits

All limits are positive integers and have compiled ceilings. Unknown limit names are rejected. A violated work bound fails the operation rather than publishing a partial candidate set or incomplete identity result. Defaults and ceilings come from [src/semantics.rs](../../src/semantics.rs) and [src/comparators/mod.rs](../../src/comparators/mod.rs).

| Limit | Default | Absolute ceiling |
| --- | ---: | ---: |
| `max_block_records` | 10,000 | 1,000,000 |
| `warning_block_records` | Smaller of 5,000 and `max_block_records` | Cannot exceed `max_block_records` |
| `max_candidate_pairs` | 1,000,000 | 100,000,000 |
| `max_comparator_work` | 1,000,000 | 100,000,000 |
| `max_decision_closure` | 10,000 | 1,000,000 |
| `max_active_records` | 100,000 | 10,000,000 |
| `max_automatic_edges` | 1,000,000 | 100,000,000 |
| `max_records_per_component` | 100,000 | 10,000,000 |
| `max_component_checks` | 1,000,000 | 100,000,000 |

These numbers are correctness/work bounds, not recommended deployment sizes or performance guarantees. Candidate density, component shape, source capture, retained history, and publication costs all matter. Qualify representative data before increasing a bound, and use [Troubleshooting](../how-to/troubleshoot.md) to distinguish an oversized candidate block from a comparator or resolver limit.

## Implementation sources

The defining checks live in [constructors](../../src/api/constructors.rs), [definition validation](../../src/definition/validate.rs), [source validation](../../src/definition/source.rs), [candidate parsing](../../src/candidate.rs), [cleaners](../../src/cleaners/), and [golden selection](../../src/golden.rs). The user-facing recipes above are based on those implemented contracts; future capabilities in the design documents are not implicitly accepted definition options.

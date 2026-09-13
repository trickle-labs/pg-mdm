# V1 acceptance register

This register maps every acceptance paragraph in [`DESIGN_V1.md`](../DESIGN_V1.md#17-acceptance-criteria)
to executable evidence or an owned v1.0 blocker. A blocker means the claim is
not qualified by v0.12; it must stay open until its owner adds and passes the
listed test. `scripts/run_e2e_tests.sh` is the PostgreSQL/Graph V1 command.

## Evidence identity

The quality run is retained in
[`evidence/v0.12/organization-quality.json`](../evidence/v0.12/organization-quality.json)
with its full log at `evidence/v0.12/organization-quality.log`. The Docker run
is retained in [`evidence/v0.12/docker-e2e-envelope.json`](../evidence/v0.12/docker-e2e-envelope.json)
with `evidence/v0.12/docker-e2e.log` and the package file manifest. Each report
contains the tested source commit and exact input digests. The upstream graph
input is pg_trickle 0.105.2, artifact SHA-256
`bf8d8dcff728a5cf9e09458b70f2c3ce2c92cc109f4e7c79ae2933ac8bb03416`.

## Register

| ID | `DESIGN_V1.md` acceptance claim | Executable test and command | v0.12 result | v1.0 blocker and owner |
|---|---|---|---|---|
| A1 | Define entities over supported local/partitioned tables; tracked and soft-delete sources; scalar/composite keys; built-in cleaners; exact/fuzzy and strong/supporting rules; inspect candidate plan. | `mdm.describe` / source-definition checks in `tests/e2e.sql`; `test_all_fixture_cases` (`cargo test --test normalization_tests --features pg18`); `candidate_graph_has_separate_limit_and_pair_stages` and `composite_candidate_keys_are_length_delimited_bytea` (`cargo test --test e2e_candidate_spec_tests --features pg18`). | Partial: the E2E customer path is tracked, scalar-key and exact-email; unit fixtures cover normalization and candidate encoding. | Add runtime E2E coverage for local plus partitioned sources, soft-delete, composite keys, fuzzy comparison, and supporting evidence. Owner: pg-mdm API/integration-test maintainer. Blocks v1.0. |
| A2 | Bound candidate work, fail closed on incomplete work, resolve conservative components without all-pairs comparison, and publish stable IDs and three SQL outputs. | `generated_exact_candidates_match_independent_oracle` (`cargo test --lib --features pg18 generated_exact_candidates_match_independent_oracle --offline`); `admission_is_conservative_and_deterministic`, `cannot_links_reject_automatic_edges_and_manual_contradictions_fail`, `limits_fail_closed` (`cargo test --test resolver_tests --features pg18`); candidate AUTO/FULL and limit rollback cases in `tests/e2e.sql` (`scripts/run_e2e_tests.sh`). | Passed for the tested fixtures and PostgreSQL paths. | None for the tested V1 contract. New source types or modes remain outside this evidence. |
| A3 | Persist MATCH/NOT_MATCH and anchored golden overrides with optimistic concurrency and explanations. | `mdm_steward.decide` is exercised for MATCH, NOT_MATCH, replacement, stale-version rejection, and contradiction in `tests/e2e.sql`; command `scripts/run_e2e_tests.sh`. Pure decision/review/override semantics: `cargo test --test decision_tests --test review_tests --test golden_tests --features pg18` and `cargo test --lib --features pg18 identity --offline`. | Decision SQL and pure lifecycle semantics pass; the golden override SQL entry points are not exercised end to end. | Add PostgreSQL role/concurrency tests for `mdm_steward.override_golden` and `mdm_steward.clear_golden_override`, including stale versions and durable audit rows. Owner: pg-mdm API/integration-test maintainer. Blocks v1.0. |
| A4 | Enforce cannot-link closure and visible evidence precedence; prevent singleton chain accretion; preserve IDs on no-op, merge, split, and rebuild; explain retired IDs. | Resolver tests above; `set_oracle_matches_union_find_on_a_small_graph` and `generated_small_graphs_match_oracle_and_ignore_input_order` (`cargo test --lib --features pg18 resolver::tests --offline`); refresh/no-op/merge/split/rebuild public-output checks in `tests/e2e.sql` (`scripts/run_e2e_tests.sh`). | Passed for those paths. | Add runtime `mdm.explain()` checks for active and retired IDs, including its machine-readable facts and history limits. Owner: pg-mdm API/integration-test maintainer. Blocks v1.0. |
| A5 | Qualify supported pg_trickle capability, graph contracts, role/RLS behavior, external orchestration, strict refresh, source boundary, atomic publication, concurrency, rollback, deletes, contract change, unsupported versions, clone, and restore. | Named E2E assertion blocks in `tests/e2e.sql` and `scripts/run_e2e_tests.sh`; the retained log and exact package/build identity are in the Docker evidence report. | Passed for capability, public graph contract, roles/RLS, source boundary, publication, rollback/retry, deletes, graph change, unsupported versions, clone, and restore. | Extend shared PGT conformance with refresh-vs-refresh and refresh-vs-drop/lifecycle races. Owner: pg_trickle conformance-suite maintainer with pg-mdm integration-test maintainer. Blocks v1.0. |
| A6 | Replay generated semantic sequences through differential and clean full graph maintenance and require equal terminal evidence and published results at each boundary. | `generated_small_graphs_match_oracle_and_ignore_input_order` exercises the pure resolver; AUTO/FULL graph probes compare exact rows in `tests/e2e.sql`; command: `scripts/run_e2e_tests.sh`. | Partial: probe relations pass, while production candidate nodes intentionally remain `FULL`; there is no production differential replay. | Add generated insert/update/delete, decision, edge appearance/disappearance, merge/split, golden-only, and full-fallback sequences through production graphs. Owner: pg_trickle conformance-suite maintainer. Blocks v1.0. |
| A7 | Use an internal labeled domain to report candidate recall, false merges, missed matches, cluster errors, review volume, and safety cases. | `organization_fixture_report_uses_production_candidate_and_resolution_paths`; command `just test-organization-quality-evidence`. | Passed the frozen synthetic regression thresholds. Held-out: 3/3 labeled positive pairs found; zero false merges, missed matches, and cluster errors; 1 review; all 9 safety cases had zero false merges. Fixture SHA-256 and limits are in the report. | Replace or supplement synthetic labels with a domain-reviewed reference set before making production accuracy claims or changing defaults. Owner: release/domain-quality owner. This limits claims; it does not block v0.12 regression qualification. |

## Open v1.0 blockers

The blocking test work is assigned to the owning engineering roles above. The
named people filling those roles should be recorded in the v1.0 release issue
before the corresponding blocker is closed. Do not treat a pure Rust test as a
substitute for a missing PostgreSQL/API integration test.

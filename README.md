# pg_mdm

**Turn scattered PostgreSQL records into customers, companies, and other entities you can trust.**

Your CRM knows a customer as “Acme Ltd.” Billing knows the same customer as “ACME LIMITED.” Both records matter, but applications need a dependable way to recognize the customer, choose useful values, and explain where those values came from. pg_mdm brings that work into PostgreSQL: you describe your sources and matching rules in SQL, refresh the entity, and query ordinary tables containing **golden records**—the selected field values applications should use for each resolved entity. Companion tables show which source records belong together and which cases need a person's judgment.

pg_mdm combines deterministic matching with durable identity. A customer keeps its `mdm_id` through an ordinary name change, selected values retain their provenance, and a failed refresh leaves the previous complete publication available. [`pg_trickle`](https://github.com/trickle-labs/pg-trickle) maintains the changing relational evidence; pg_mdm uses that evidence, your policies, and steward decisions to resolve entities. You can begin with exact matching on a small dataset and grow into controlled definition changes, conservative fuzzy matching, stewardship, and operational recovery without leaving SQL.

> [!IMPORTANT]
> The current version is **0.15.0, a demo preview for synthetic/demo data only**, targeting PostgreSQL 18. The [release](https://github.com/trickle-labs/pg-mdm/releases/tag/v0.15.0) is not production-qualified: **do not connect it to a real workload or enable live controls**. The [release qualification gates](https://github.com/trickle-labs/pg-mdm/issues/28) remain blocked. Use a disposable learning environment; the advanced guides describe synthetic-data rehearsals and implemented contracts. The [compatibility reference](docs/reference/compatibility.md) identifies the build inputs and separates current availability from historical evidence.

## Start here

[Install a local learning environment](docs/how-to/install.md), then follow [Your first customer golden records](docs/tutorials/first-entity.md). The tutorial supplies four customer records, takes you through each SQL statement, and shows the expected result: three customers, including one correctly combined pair. You will then change a source record and see that the customer's identity remains stable. Allow about 30 minutes for the lesson after the environment is ready; building the extension image takes additional time.

Already have an installation? Start at the [documentation home](docs/README.md) and choose the route that matches your task. The documentation follows [Diátaxis](https://diataxis.fr/): tutorials help you learn through a complete exercise, how-to guides help you finish a particular job, reference pages give precise contracts, and explanations help you understand the design.

| Your goal | Where to go |
| --- | --- |
| Learn the complete first workflow | [Build your first entity](docs/tutorials/first-entity.md) |
| Move beyond automatic matching | [Inspect and refine a golden record](docs/tutorials/inspect-and-refine.md) |
| Model representative source tables | [Model sources and matching rules](docs/how-to/model-sources.md) |
| Change a definition confidently | [Preview and publish a definition change](docs/how-to/change-a-definition.md) |
| Resolve an exception | [Steward records and golden values](docs/how-to/steward-records.md) |
| Run and recover an installation | [Operate, upgrade, and restore](docs/how-to/operate.md) |
| Build a policy integration | [Automate stewardship controls](docs/how-to/automate-stewardship.md) |
| Look up an argument or a result | [SQL API](docs/reference/sql-api.md), [definitions](docs/reference/definitions.md), and [outputs](docs/reference/outputs.md) |
| Diagnose a problem | [Troubleshooting](docs/how-to/troubleshoot.md) |

## How it works

You define an **entity**, such as `customer`, using five building blocks: `source`, `field`, `match`, `golden_value`, and `entity`. A source maps your existing columns to shared fields. Cleaners make values comparable without rewriting the source. Match rules describe which agreements count as evidence, and golden-value policies choose what applications should read. The [first tutorial](docs/tutorials/first-entity.md) provides a complete definition you can run, including the roles and grants it needs.

After the first successful refresh, each entity has three primary outputs:

| Table | What it answers |
| --- | --- |
| `mdm_out.customer` | “What do we currently know about each customer?” |
| `mdm_out.customer_members` | “Which source records belong to this customer, including retained inactive membership?” |
| `mdm_out.customer_review` | “Which ambiguities or conflicts need attention?” |

These are PostgreSQL tables, so applications can join them with existing data and receive explicitly granted read access. Operators use `mdm.describe()` to inspect state, `mdm.preview()` to examine a desired definition, `mdm.refresh()` to publish changes, and `mdm.explain()` to investigate the evidence behind a result. Source changes become visible in the outputs when a refresh succeeds; arrange refresh calls as part of your application or operational schedule.

## What makes the design different

Good entity resolution starts with the meaning of your data. A shared billing email can identify one customer in a carefully chosen dataset and a whole household in another. pg_mdm makes the rules and consequences visible: candidate generation has an explicit boundary, uncertain component growth can produce a review instead of a merge, and human decisions carry reasons and versions. Read [How entity resolution works](docs/explanation/entity-resolution.md) before broadening your first rules, then use [Publication and durable identity](docs/explanation/publication-and-identity.md) to understand changes over time.

The current implementation also has boundaries that matter when you adopt it. Golden source preference currently follows canonical source-name order, rather than the order of the `golden_value.sources` array. Incremental resolution can reduce some work, while publication assembly and other stages still perform global work. Policy automation has explicit binding and restart activation requirements. These details are covered beside the relevant examples and in the reference pages, so you can make an informed choice before expanding a deployment.

## Install v0.15 candidate

The complete, current procedure is in [Install pg_mdm](docs/how-to/install.md). It includes a Docker learning environment, a native installation route, the protected helper owner, and an installation check. For an older checkout, use the documentation shipped with that version. The root [dependency record](DEPENDENCIES.md) contains exact build pins.

## Define and refresh an entity

Use the [first-entity tutorial](docs/tutorials/first-entity.md) for a complete runnable example and [Change a definition](docs/how-to/change-a-definition.md) for an existing deployment. `mdm.create()` records the desired definition; the successful refresh activates and publishes it.

## Roll out compiler v11

The [compiler rollout procedure](docs/how-to/operate.md#roll-out-a-compiler-or-producer-change) retains the canary, full-refresh fallback, delta, and evidence checks from this repository's earlier README. Advanced operators can follow the linked qualification evidence without making it a prerequisite for the first tutorial.

## Rebind restored entities

Follow [Restore and rebind](docs/how-to/operate.md#restore-and-rebind). Restored definitions need database-local role and relation bindings rebuilt under the correct identity before normal entity operations resume.

## Project status

The [changelog](CHANGELOG.md), [dependency record](DEPENDENCIES.md), and [compatibility reference](docs/reference/compatibility.md) describe the current implementation and its provenance. [DESIGN_V1.md](DESIGN_V1.md), [DESIGN_V2.md](DESIGN_V2.md), and [ROADMAP.md](ROADMAP.md) retain design proposals, future capabilities, and release gates; they are useful context, but a proposed feature is not an installed API.

## Contributing

Useful contributions include concrete matching failures, clearer explanations, reproducible installation problems, and recovery cases. When reporting a problem, include the extension versions, source and binary identities when available, the smallest reproducible definition, and the relevant result code. Remove personal data before sharing a fixture. See [Troubleshooting](docs/how-to/troubleshoot.md#prepare-a-useful-problem-report) for a practical checklist. For implementation work, follow [AGENTS.md](AGENTS.md) and use the existing [test commands](justfile) and [CI workflow](.github/workflows/ci.yml).

## License

pg_mdm is licensed under the [Apache License 2.0](LICENSE).

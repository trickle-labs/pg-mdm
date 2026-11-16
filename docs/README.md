# pg_mdm documentation

pg_mdm helps you turn records from multiple PostgreSQL sources into a dependable view of the entities they describe. This documentation is designed to stay useful as your questions change: first you want a working result, then you want to adapt it to your data, and eventually you need to understand why a match happened or how to recover an installation. Start with the learning path below, or go directly to the task or reference you need.

These pages describe the **0.15.0 demo preview for synthetic/demo data only**. PostgreSQL 18 and the pinned pg_trickle build are the learning baseline. The [release](https://github.com/trickle-labs/pg-mdm/releases/tag/v0.15.0) forbids connecting this preview to real workloads or enabling live controls, and its [qualification gates remain blocked](https://github.com/trickle-labs/pg-mdm/issues/28). The advanced guides explain current contracts and synthetic-data rehearsals; they do not authorize production use. Read the [compatibility and status reference](reference/compatibility.md) for the full boundary.

## A path from first result to advanced use

| Stage | What you will be able to do | Read next |
| --- | --- | --- |
| 1. Get a result | Install a disposable environment and resolve four source records into three customers. | [Install](how-to/install.md), then [Your first entity](tutorials/first-entity.md) |
| 2. Understand a result | Inspect matching evidence and value policies, then apply and clear an override. | [Inspect and refine](tutorials/inspect-and-refine.md) |
| 3. Model your domain | Choose stable keys, normalize fields, bound candidates, and preview a change with synthetic fixtures. | [Model sources](how-to/model-sources.md) and [Change a definition](how-to/change-a-definition.md) |
| 4. Rehearse operations | Practice application grants, refresh observation, upgrade, restore, and troubleshooting. | [Operate pg_mdm](how-to/operate.md) and [Security model](explanation/security.md) |
| 5. Integrate and investigate | Use versioned steward actions, policy receipts, and retained explanations. | [Steward records](how-to/steward-records.md), [Automate controls](how-to/automate-stewardship.md), and [SQL reference](reference/sql-api.md) |

## Tutorials: learn by doing

Tutorials guide you through a complete exercise with prepared data and expected outcomes. Follow the steps in order in a disposable database; once you have seen the workflow work, the how-to guides show how to adapt it.

- [Your first customer golden records](tutorials/first-entity.md): create, refresh, query, and retain identity through a source change.
- [Inspect and refine a golden record](tutorials/inspect-and-refine.md): explore the evidence, make a reasoned override, publish it, and return to automatic selection.

## How-to guides: finish a task

Choose the guide that matches the result you need. Each guide identifies its prerequisites, uses explicit role boundaries, and links to reference details when an argument or restriction needs a closer look.

- [Install a local learning environment or native package](how-to/install.md).
- [Model source tables, cleaning, and matching](how-to/model-sources.md).
- [Preview and publish a definition change](how-to/change-a-definition.md).
- [Steward records and golden values](how-to/steward-records.md).
- [Operate, upgrade, restore, and retire an entity](how-to/operate.md).
- [Automate stewardship controls](how-to/automate-stewardship.md).
- [Troubleshoot installation, matching, and refresh problems](how-to/troubleshoot.md).

## Reference: look up a contract

Reference pages describe the current SQL surface and observable results. Use them while writing a definition or integration; unlike a tutorial, they are organized for lookup and do not need to be read in sequence.

- [SQL API](reference/sql-api.md): constructors, core actions, administrative actions, steward actions, and policy functions.
- [Definition reference](reference/definitions.md): source contracts, cleaners, candidate predicates, golden policies, and limits.
- [Output and result reference](reference/outputs.md): table columns, refresh diagnostics, explanation subjects, and policy relations.
- [Compatibility and project status](reference/compatibility.md): version pins, preview boundaries, and evidence provenance.

## Explanation: understand the decisions

Explanations connect the commands you run to the behavior you observe. They are especially useful before choosing broader matching rules, designing downstream integrations, or deciding how to handle an operational exception.

- [How entity resolution works](explanation/entity-resolution.md).
- [Publication, incremental work, and durable identity](explanation/publication-and-identity.md).
- [The security and ownership model](explanation/security.md).

## Design and implementation material

The repository retains [design proposals](../DESIGN_V1.md), a [future capability catalogue](../DESIGN_V2.md), the [roadmap](../ROADMAP.md), and [acceptance contracts](acceptance-contracts/pg-mdm-m2.md). These explain intentions and qualification requirements, some of which are not current capabilities. The [changelog](../CHANGELOG.md), [dependency record](../DEPENDENCIES.md), executable tests, and current API implementation provide the version-specific context.

The four sections follow [Diátaxis](https://diataxis.fr/). When contributing a page, decide whether the reader is learning, completing a task, looking up a fact, or seeking understanding, then keep the page focused on that need. This lets the tutorial remain approachable while the advanced material stays precise.

[Back to the project](../README.md)

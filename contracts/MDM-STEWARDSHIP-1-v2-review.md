# MDM-STEWARDSHIP/1 revision 2 amendment record

Canonical agreement: [`MDM-STEWARDSHIP-1.json`](MDM-STEWARDSHIP-1.json),
revision 2, SHA-256
`6ab1540836c1477908488fb706584367f75a1abee10ffb40fa3e96e243f7a70d`.

Revision 1 is preserved byte-for-byte at
[`revisions/MDM-STEWARDSHIP-1-v1.json`](revisions/MDM-STEWARDSHIP-1-v1.json),
SHA-256 `2161ce22b9d924ff3f3e6d8acde62fed01b6b1c1c4d7ba0fd4d5eb6e396e0fcf`.
The v1 fixture is unchanged at SHA-256
`900f365533bab13480fcb88ab8e8b7956beb4f14a8cfc92cd46770b2e26ef3e4`.

The requesting contract owner authorized this material amendment on
2026-09-23 and confirmed approval to change the shared contract. Personal
approver names were not supplied. The prior joint sign-off remains attached to
revision 1; this note records the new authorization without rewriting it.

| Affected agreement | Revision 1 | Revision 2 |
|---|---|---|
| Binding scope and limits | One active binding per scope; no bound queue, due interval, or escalation maximum. | One unreplaced binding per entity and automation role; canonical action and queue limits, due interval, escalation maximum, and optimistic versions. |
| Administration | Register, pause, and replace under the MDM administrator path; replacement is active. | Entity execution role uses versioned create, replace, and state APIs; replacement is inactive until explicitly activated. |
| Automation role | Non-superuser and NOBYPASSRLS. | NOLOGIN, NOSUPERUSER, NOBYPASSRLS, no membership path to helper/entity execution roles, and name/OID authentication before receipt lookup. |
| Existing installations | Revision 1 administration remains available. | Retire revision 1 administration APIs; retain bindings and receipts, pause existing runtime, and require bounded replacement plus explicit activation. |

The pg-react v0.47 qualification continues to refer to revision 1 and its exact
historical evidence. Revision 2 governs M2 implementation. The read-only v0.47
adapter does not implement an M2 write worker; actual-worker privilege and
work/request/receipt correlation remain qualification gaps in M2 R2 and R14.

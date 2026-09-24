# MDM-STEWARDSHIP/1 revision 3 amendment record

Canonical contract: `contracts/MDM-STEWARDSHIP-1.json`, revision 3, SHA-256
`fd6f5e6ceb7c65fc93924c004a24f348e4296f2d83e83ce4e82cea6493e77efe`.
Revision 2 is preserved at
[`revisions/MDM-STEWARDSHIP-1-v2.json`](revisions/MDM-STEWARDSHIP-1-v2.json).

On 2026-09-23, the requesting contract owner approved this adjustment after the
Rust-facing `name[]` argument crashed during an E2E API call. Revision 2 used
`name[]` for `allowed_queues` on create and replace. Revision 3 uses `text[]`
for those inputs, validates and stores the values in the existing durable
`name[]` column. The public intent API and durable queue representation do not
change.

# Policy binding pause / delivery barrier

The real downstream React regression reproduced an unauthorized control on the
original MDM binary: a claimed delivery held its binding lock while waiting for
the case row; the authorized pause returned; then delivery applied `ESCALATE`.
The baseline source is MDM `6ad23ca11cb956f264e93e46a6351d32ea4a1ca8`, equivalent
to current main `5fdee89b589da8fb3ab86157cc716a2aaff8bdf8` for this implementation.
The exact selected producer is `c7e257e551d6f6bacb84ecfe6f5daf7637c53673`, archive
`d6233d49cee80d33ac30626e0af7932a9698279233b8fceef3447275192abc27`.
The repository's default historical producer pin remains unchanged.

Two SQL lock changes repair the shared MDM authority:

- `persist_set_policy_binding_state` locks the unreplaced binding before changing
  runtime state. Pause acknowledgment therefore follows any earlier delivery.
- `submit_intent` takes that binding lock during authenticated binding lookup,
  before the separate existing eligibility query. If it waits behind a pause,
  its later READ COMMITTED query observes the committed runtime state rather
  than an old join snapshot. Both paths retain binding-before-case ordering.

There is no schema, public signature, request codec, receipt semantic, source pin
or activation-default change. Rust line locations remain unchanged, preserving
the frozen packaged SQL archive.

`tests/policy_pause_barrier.sql` uses asynchronous dblink sessions and observed
`pg_blocking_pids`, with bounded statement/barrier deadlines. It fails on the
original binary before releasing the binding lock, then passes on the repair.
It compares complete binding, runtime, case and receipt state. The standard E2E
runner executes it after the existing policy fixtures.

The paired React regression executes native typed callbacks from real claimed
episodes. It covers delivery-first and pause-first waits, committed-pause-before-
claim, human edits in both claim orders, replacement reducing the permitted
level, exact canonical requests/receipts, terminal outcomes, fresh resumption,
cross-key applied-level denial and healthy subsequent managed cycles.

That regression also revealed a separate React hold issue: a retained
`BINDING_PAUSED` outcome prevented fresh work after authorized resumption. The
downstream repair preserves the old immutable request/receipt and applies only
to a new work identity after current eligibility checks. It is not part of MDM's
receipt deduplication authority.

The accompanying manifest retains exact library/source hashes, commands,
original failures and passing vectors. Local binaries are Linux amd64 under
ARM64 emulation. Full packaged CI and release qualification remain required;
this work does not authorize release, merge or production activation.

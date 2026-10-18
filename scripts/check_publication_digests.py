"""Independent canonical-publication oracle over raw durable E2E snapshots."""

import hashlib
import json


def publication_projection(state):
    revision = state["publication_revision"]
    memberships = sorted(state["memberships"], key=lambda r: (
        bytes.fromhex(r["source_id"]["source_record_key"]), r["source_record_id"]))
    reviews = sorted(state["reviews"], key=lambda r: (r["issue_key"], r["occurrence"]))
    golden = sorted((r for r in state["golden_provenance"] if r["publication_revision"] == revision),
                    key=lambda r: (r["mdm_id"], r["field_name"]))
    facts = sorted([r["subject_key"], r["fact_kind"], r["fact"]["reason_code"], r["fact"]["evidence_groups"]]
                   for r in state["resolution_facts"] if r["publication_revision"] == revision)
    # PostgreSQL's JSON encoding of bytea is hexadecimal with a leading backslash-x.
    for fact in facts:
        fact[0] = bytes.fromhex(fact[0][2:]).decode("utf-8")
    facts.sort()
    return {
        "identity": {
            "registry": [[r["mdm_id"], r["status"]]
                         for r in sorted(state["identity_registry"], key=lambda r: r["mdm_id"])],
            "memberships": [[r["source_record_id"], list(bytes.fromhex(r["source_id"]["source_record_key"])),
                             r["mdm_id"], r["active"], r["first_membership_revision"],
                             r["last_membership_revision"], r["membership_reason"]] for r in memberships],
            "aliases": [[r["alias_mdm_id"], r["canonical_mdm_id"]] for r in sorted(state["identity_aliases"],
                key=lambda r: (r["alias_mdm_id"], r["publication_revision"], r["canonical_mdm_id"]))],
            "splits": [[r["parent_mdm_id"], r["child_mdm_id"]] for r in sorted(state["identity_splits"],
                key=lambda r: (r["parent_mdm_id"], r["publication_revision"], r["child_mdm_id"]))],
        },
        "reviews": [[list(bytes.fromhex(r["issue_key"][2:])), r["occurrence"], r["status"], r["severity"],
                     r["reason_code"], r["subjects"], r["masked_summary"]] for r in reviews],
        "golden": [[r["mdm_id"], r["field_name"], r["value"], r["normalized_value"], r["status"],
                    r["winning_source_record_id"], r["policy"], r["policy_version"], r["tie_break"],
                    r["contributors"]] for r in golden],
        "resolution_facts": facts,
    }


def check_cases(cases):
    checked = []
    for case in cases:
        state = case["state"]
        projection = publication_projection(state)
        payload = json.dumps(projection, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
        tag = b"pg_mdm/publication/v1"
        expected = hashlib.sha256(len(tag).to_bytes(4, "big") + tag + len(payload).to_bytes(4, "big") + payload).hexdigest()
        publication = next(r for r in state["publications"] if r["publication_revision"] == state["publication_revision"])
        actual = publication["result_digest"][2:]
        if actual != expected:
            raise AssertionError(f"{case['label']} canonical digest mismatch: expected {expected}, actual {actual}")
        checked.append({"label": case["label"], "result_digest": actual})
    return checked


if __name__ == "__main__":
    import sys
    from pathlib import Path

    print(json.dumps(check_cases(json.loads(Path(sys.argv[1]).read_text())), indent=2))

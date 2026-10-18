-- Global publication assembly; evaluation itself has already used scoped typed state.
WITH records AS (
    SELECT value::uuid AS id FROM pg_catalog.jsonb_array_elements_text($3::jsonb)
), mdm_ids AS (
    SELECT value::uuid AS id FROM pg_catalog.jsonb_array_elements_text($4::jsonb)
), registry AS (
    SELECT pg_catalog.jsonb_build_array(mdm_id, status) AS row
    FROM mdm_internal.identity_registry
    WHERE entity_id = $1::uuid AND mdm_id NOT IN (SELECT id FROM mdm_ids)
    UNION ALL
    SELECT value FROM pg_catalog.jsonb_array_elements($5::jsonb #> '{identity,registry}')
), memberships AS (
    SELECT pg_catalog.jsonb_build_array(m.source_record_id,
        (SELECT pg_catalog.jsonb_agg(pg_catalog.get_byte(r.source_record_key, n) ORDER BY n)
         FROM pg_catalog.generate_series(0, pg_catalog.octet_length(r.source_record_key) - 1) n),
        m.mdm_id, m.active, m.first_membership_revision, m.last_membership_revision,
        m.membership_reason) AS row
    FROM mdm_internal.memberships m
    JOIN mdm_internal.source_records r USING (entity_id, source_record_id)
    WHERE m.entity_id = $1::uuid AND m.source_record_id NOT IN (SELECT id FROM records)
    UNION ALL
    SELECT value FROM pg_catalog.jsonb_array_elements($5::jsonb #> '{identity,memberships}')
), aliases AS (
    SELECT alias_mdm_id, canonical_mdm_id, publication_revision
    FROM mdm_internal.identity_aliases WHERE entity_id = $1::uuid
    UNION
    SELECT (value->>0)::uuid, (value->>1)::uuid, (value->>2)::bigint
    FROM pg_catalog.jsonb_array_elements($5::jsonb->'aliases')
), splits AS (
    SELECT parent_mdm_id, child_mdm_id, publication_revision
    FROM mdm_internal.identity_splits WHERE entity_id = $1::uuid
    UNION
    SELECT (value->>0)::uuid, (value->>1)::uuid, (value->>2)::bigint
    FROM pg_catalog.jsonb_array_elements($5::jsonb->'splits')
), reviews AS (
    SELECT pg_catalog.jsonb_build_array(
        (SELECT pg_catalog.jsonb_agg(pg_catalog.get_byte(issue_key, n) ORDER BY n)
         FROM pg_catalog.generate_series(0, 31) n),
        occurrence, status, severity, reason_code, subjects, masked_summary) AS row
    FROM mdm_internal.reviews
    WHERE entity_id = $1::uuid AND NOT pg_catalog.jsonb_path_exists(
        subjects, '$.** ? (@ == $ids[*])', pg_catalog.jsonb_build_object('ids', $3::jsonb || $4::jsonb))
    UNION ALL
    SELECT value FROM pg_catalog.jsonb_array_elements($5::jsonb->'reviews')
), golden AS (
    SELECT pg_catalog.jsonb_build_array(mdm_id, field_name::text, value, normalized_value,
        status, winning_source_record_id, policy, policy_version, tie_break, contributors) AS row
    FROM mdm_internal.golden_provenance
    WHERE entity_id = $1::uuid AND publication_revision = $2
        AND mdm_id NOT IN (SELECT id FROM mdm_ids)
    UNION ALL
    SELECT value FROM pg_catalog.jsonb_array_elements($5::jsonb->'golden')
), facts AS (
    SELECT pg_catalog.jsonb_build_array(pg_catalog.convert_from(subject_key, 'UTF8'),
        fact_kind, fact->>'reason_code', fact->'evidence_groups') AS row
    FROM mdm_internal.resolution_facts
    WHERE entity_id = $1::uuid AND publication_revision = $2 AND subject_kind = 'pair'
        AND NOT (pg_catalog.split_part(pg_catalog.convert_from(subject_key, 'UTF8'), ':', 1)::uuid IN (SELECT id FROM records)
             AND pg_catalog.split_part(pg_catalog.convert_from(subject_key, 'UTF8'), ':', 2)::uuid IN (SELECT id FROM records))
    UNION ALL
    SELECT value FROM pg_catalog.jsonb_array_elements($5::jsonb->'resolution_facts')
)
SELECT pg_catalog.jsonb_build_object(
    'identity', pg_catalog.jsonb_build_object(
        'registry', COALESCE((SELECT pg_catalog.jsonb_agg(row ORDER BY (row->>0) COLLATE "C") FROM registry), '[]'::jsonb),
        'memberships', COALESCE((
            SELECT pg_catalog.jsonb_agg(m.row ORDER BY r.source_record_key, r.source_record_id)
            FROM memberships m JOIN mdm_internal.source_records r
              ON r.entity_id = $1::uuid AND r.source_record_id = (m.row->>0)::uuid
        ), '[]'::jsonb),
        'aliases', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_array(alias_mdm_id, canonical_mdm_id)
            ORDER BY alias_mdm_id, publication_revision, canonical_mdm_id) FROM aliases), '[]'::jsonb),
        'splits', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_array(parent_mdm_id, child_mdm_id)
            ORDER BY parent_mdm_id, publication_revision, child_mdm_id) FROM splits), '[]'::jsonb)
    ),
    'reviews', COALESCE((SELECT pg_catalog.jsonb_agg(row
        ORDER BY ARRAY(SELECT value::int FROM pg_catalog.jsonb_array_elements_text(row->0)), (row->>1)::int)
        FROM reviews), '[]'::jsonb),
    'golden', COALESCE((SELECT pg_catalog.jsonb_agg(row ORDER BY (row->>0) COLLATE "C", (row->>1) COLLATE "C") FROM golden), '[]'::jsonb),
    'resolution_facts', COALESCE((SELECT pg_catalog.jsonb_agg(row
        ORDER BY (row->>0) COLLATE "C", (row->>1) COLLATE "C", (row->>2) COLLATE "C",
            ARRAY(SELECT pg_catalog.jsonb_array_elements_text(row->3)) COLLATE "C") FROM facts), '[]'::jsonb)
)

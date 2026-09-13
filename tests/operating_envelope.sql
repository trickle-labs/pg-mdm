CREATE TEMP TABLE e2e_operating_envelope (report jsonb NOT NULL);

DO $envelope$
DECLARE
    entity_id uuid;
    member record;
    output_relation record;
    relation regclass;
    row_count bigint;
    block_pair_discoveries numeric;
    output_bytes bigint := 0;
    block_memberships bigint := 0;
    max_block_records bigint := 0;
    raw_pair_discoveries numeric := 0;
    candidate_pairs bigint := 0;
    comparisons bigint := 0;
    active_source_rows bigint := 0;
    history_rows bigint := 0;
    history_bytes bigint := 0;
    temp_bytes bigint := 0;
    result jsonb := '{}'::jsonb;
    outputs jsonb := '{}'::jsonb;
    last_operation jsonb := '{}'::jsonb;
    last_operation_ms bigint;
    last_operation_started timestamptz;
    last_operation_completed timestamptz;
    last_outcome jsonb;
BEGIN
    SELECT e.entity_id
      INTO entity_id
      FROM mdm_internal.entities e
     WHERE e.entity_name = 'customer';
    IF entity_id IS NULL THEN
        RAISE EXCEPTION 'customer entity missing from E2E database';
    END IF;

    SELECT count(*)
      INTO active_source_rows
      FROM mdm_internal.source_records r
     WHERE r.entity_id = entity_id AND r.active;

    FOR member IN
        SELECT DISTINCT ON (m.logical_id) m.logical_id, m.relation_name
          FROM mdm_internal.graph_bindings b
          JOIN mdm_internal.graph_members m USING (graph_binding_id)
         WHERE b.entity_id = entity_id
         ORDER BY m.logical_id, b.graph_generation DESC
    LOOP
        relation := pg_catalog.to_regclass(member.relation_name);
        IF relation IS NULL THEN
            CONTINUE;
        END IF;
        IF member.logical_id LIKE 'blocks/%' THEN
            EXECUTE pg_catalog.format('SELECT count(*) FROM %s', relation)
               INTO row_count;
            block_memberships := block_memberships + row_count;
        ELSIF member.logical_id LIKE 'block-stats/%' THEN
            EXECUTE pg_catalog.format(
                'SELECT COALESCE(sum(block_records::numeric * (block_records - 1) / 2), 0), COALESCE(max(block_records), 0) FROM %s',
                relation
            ) INTO block_pair_discoveries, row_count;
            raw_pair_discoveries := raw_pair_discoveries + block_pair_discoveries;
            max_block_records := GREATEST(max_block_records, row_count);
        ELSIF member.logical_id = 'pairs/customer' THEN
            EXECUTE pg_catalog.format('SELECT count(*) FROM %s', relation)
               INTO candidate_pairs;
        ELSIF member.logical_id = 'evidence/customer' THEN
            EXECUTE pg_catalog.format('SELECT count(*) FROM %s', relation)
               INTO comparisons;
        END IF;
    END LOOP;

    FOR output_relation IN
        SELECT * FROM (VALUES
            ('entity', 'mdm_out.customer'),
            ('members', 'mdm_out.customer_members'),
            ('reviews', 'mdm_out.customer_review')
        ) AS outputs(kind, relation_name)
    LOOP
        relation := pg_catalog.to_regclass(output_relation.relation_name);
        IF relation IS NULL THEN
            CONTINUE;
        END IF;
        EXECUTE pg_catalog.format('SELECT count(*) FROM %s', relation)
           INTO row_count;
        outputs := outputs || pg_catalog.jsonb_build_object(
            output_relation.kind,
            pg_catalog.jsonb_build_object(
                'rows', row_count,
                'bytes', pg_catalog.pg_total_relation_size(relation)
            )
        );
        output_bytes := output_bytes + pg_catalog.pg_total_relation_size(relation);
    END LOOP;

    SELECT count(*),
           pg_catalog.pg_total_relation_size('mdm_internal.publications'::regclass)
         + pg_catalog.pg_total_relation_size('mdm_internal.publication_observations'::regclass)
         + pg_catalog.pg_total_relation_size('mdm_internal.reviews'::regclass)
         + pg_catalog.pg_total_relation_size('mdm_internal.resolution_facts'::regclass)
      INTO history_rows, history_bytes
      FROM mdm_internal.publications p
     WHERE p.entity_id = entity_id;
    history_rows := history_rows
        + (SELECT count(*) FROM mdm_internal.publication_observations o WHERE o.entity_id = entity_id)
        + (SELECT count(*) FROM mdm_internal.reviews r WHERE r.entity_id = entity_id)
        + (SELECT count(*) FROM mdm_internal.resolution_facts f WHERE f.entity_id = entity_id);

    SELECT d.temp_bytes
      INTO temp_bytes
      FROM pg_catalog.pg_stat_database d
     WHERE d.datname = pg_catalog.current_database();

    SELECT o.outcome, o.started_at, o.completed_at
      INTO last_outcome, last_operation_started, last_operation_completed
      FROM mdm_internal.operations o
     WHERE o.entity_name = 'customer' AND o.operation_kind IN ('refresh', 'rebuild')
       AND o.status = 'succeeded'
     ORDER BY o.started_at DESC, o.completed_at DESC
     LIMIT 1;
    IF last_operation_started IS NOT NULL AND last_operation_completed IS NOT NULL THEN
        last_operation_ms := GREATEST(0, (EXTRACT(EPOCH FROM
            (last_operation_completed - last_operation_started)) * 1000)::bigint);
        last_operation := pg_catalog.jsonb_build_object(
            'elapsed_ms', last_operation_ms,
            'stage_timings_ms', last_outcome -> 'stage_timings_ms',
            'active_records', last_outcome -> 'active_records',
            'component_checks', last_outcome -> 'component_checks',
            'publication_revision', last_outcome -> 'publication_revision'
        );
    END IF;

    result := pg_catalog.jsonb_build_object(
        'scope', 'customer entity state after Docker E2E; temp_bytes is cumulative for foundation database',
        'postgres_build', pg_catalog.version(),
        'source_rows', active_source_rows,
        'block_memberships', block_memberships,
        'max_block_records', max_block_records,
        'repeated_pair_discoveries', GREATEST(0, raw_pair_discoveries - candidate_pairs),
        'unique_candidate_pairs', candidate_pairs,
        'evidence_comparisons', comparisons,
        'component_checks', last_outcome -> 'component_checks',
        'temporary_bytes', temp_bytes,
        'outputs', outputs,
        'output_bytes', output_bytes,
        'retained_history_rows_customer', history_rows,
        'retained_history_table_bytes_database_wide', history_bytes,
        'last_customer_refresh', last_operation
    );
    INSERT INTO e2e_operating_envelope VALUES (result);
END
$envelope$;

SELECT report FROM e2e_operating_envelope;

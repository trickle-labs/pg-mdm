ALTER TABLE mdm_internal.policy_binding_runtime ADD COLUMN activation_epoch bytea;
CREATE FUNCTION mdm_internal.policy_runtime_epoch() RETURNS bytea LANGUAGE c STABLE PARALLEL SAFE AS 'MODULE_PATHNAME', 'policy_runtime_epoch_wrapper';

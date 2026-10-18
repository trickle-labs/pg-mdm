\set ON_ERROR_STOP on
CREATE ROLE mdm_helper_owner NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE mdm_administrator NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE mdm_test_login LOGIN NOSUPERUSER NOBYPASSRLS;
GRANT mdm_administrator TO mdm_test_login WITH SET TRUE, INHERIT FALSE;
CREATE DATABASE foundation;
\connect foundation postgres
CREATE EXTENSION pg_trickle VERSION '0.108.3';
CREATE EXTENSION pg_mdm VERSION '0.14.1';
ALTER DATABASE foundation SET jit = off;
GRANT USAGE ON SCHEMA public, mdm, mdm_admin, mdm_steward, pgtrickle TO mdm_administrator;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA pgtrickle TO mdm_administrator;
GRANT SELECT ON ALL TABLES IN SCHEMA pgtrickle TO mdm_administrator;
GRANT EXECUTE ON FUNCTION mdm_admin.verify_installation() TO mdm_administrator;
\set helper_owner mdm_helper_owner
\ir /sql/configure_helper.sql

# Build your first customer golden records in 30 minutes

In this lesson, we will combine customer records from a CRM and a billing system.
We will match one duplicate customer by email, choose the CRM's name, and update
that name without changing the customer's identity.

You need basic SQL knowledge and a terminal. You do not need Rust knowledge.
Allow 30 minutes after completing the prerequisites below.

## Before you start

Use a disposable PostgreSQL 18 instance with the pg_mdm v0.12.0 and
pg_trickle 0.105.2 extension files installed. PostgreSQL must have
`pg_trickle` in `shared_preload_libraries` and have restarted after that change.
Follow the [installation instructions](README.md#install-v012) if needed.
Installation time is outside this lesson.

Open a terminal at the root of this repository. Create a fresh database, then
connect with `psql` as your PostgreSQL superuser. These commands assume that the
administrator is named `postgres` and your local connection settings are ready:

```sh
createdb -U postgres pg_mdm_tutorial
psql -X -U postgres -d pg_mdm_tutorial
```

Keep this `psql` session open throughout the lesson. Run the SQL blocks in order.
Commands beginning with a backslash are `psql` commands.

The lesson creates three roles whose names start with `tutorial_`. Use an
instance where those roles do not already exist. We will use
`SET SESSION AUTHORIZATION` to act as a restricted login in this administrator
session. In an application, connect as the application login directly.

## 0–5 minutes: prepare the database

First, stop on SQL errors and use compact output so your results match this page:

```sql
\set ON_ERROR_STOP on
\pset format unaligned
\pset fieldsep |
\pset footer off

CREATE EXTENSION pg_trickle;
CREATE EXTENSION pg_mdm;

SELECT extname, extversion
FROM pg_extension
WHERE extname IN ('pg_mdm', 'pg_trickle')
ORDER BY extname;
```

The final query returns:

```text
extname|extversion
pg_mdm|0.12.0
pg_trickle|0.105.2
```

Create a protected helper owner, an action role, and a restricted login:

```sql
CREATE ROLE tutorial_helper NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE tutorial_admin NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE tutorial_login LOGIN NOSUPERUSER NOBYPASSRLS;
GRANT tutorial_admin TO tutorial_login WITH SET TRUE, INHERIT FALSE;

\set helper_owner tutorial_helper
\i sql/configure_helper.sql

GRANT USAGE ON SCHEMA mdm, mdm_admin TO tutorial_admin;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA mdm TO tutorial_admin;
GRANT EXECUTE ON FUNCTION mdm_admin.verify_installation() TO tutorial_admin;
GRANT USAGE ON SCHEMA pgtrickle TO tutorial_admin;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA pgtrickle TO tutorial_admin;

SET SESSION AUTHORIZATION tutorial_login;
SET ROLE tutorial_admin;
SELECT mdm_admin.verify_installation()::uuid IS NOT NULL AS installation_recorded;
```

After the setup messages, expect:

```text
installation_recorded
t
```

The check returns an operation UUID on success. We check that it is present
because its value differs between runs.

The helper owns pg-mdm's protected objects. The action role runs our entity
operations. Keep the helper role separate from the login and action role.

If the include command cannot find `sql/configure_helper.sql`, use `\cd` to
change to the repository root and repeat the include. If the check reports
`MDM_UNAUTHORIZED`, verify both identities:

```sql
SELECT session_user, current_user;
```

Expected:

```text
session_user|current_user
tutorial_login|tutorial_admin
```

`SET ROLE` alone does not change `session_user`. Both identities must pass
pg-mdm's checks.

## 5–10 minutes: load four customer records

Return to the administrator to create the source tables. Each source gets a
primary key and a timestamp for its last change:

```sql
RESET ROLE;
RESET SESSION AUTHORIZATION;

CREATE SCHEMA tutorial;
CREATE TABLE tutorial.crm_customer (
	id bigint PRIMARY KEY,
	display_name text NOT NULL,
	email_address text NOT NULL,
	updated_at timestamptz NOT NULL
);
CREATE TABLE tutorial.billing_customer (
	id bigint PRIMARY KEY,
	legal_name text NOT NULL,
	email text NOT NULL,
	updated_at timestamptz NOT NULL
);

INSERT INTO tutorial.crm_customer VALUES
	(1, 'Acme Ltd', 'accounts@acme.example', '2026-01-01 09:00:00+00'),
	(2, 'Birch Studio', 'hello@birch.example', '2026-01-01 09:00:00+00');
INSERT INTO tutorial.billing_customer VALUES
	(101, 'ACME LIMITED', ' ACCOUNTS@ACME.EXAMPLE ', '2026-01-02 09:00:00+00'),
	(102, 'Cedar Works', 'hello@cedar.example', '2026-01-02 09:00:00+00');

GRANT USAGE ON SCHEMA tutorial TO tutorial_admin;
GRANT SELECT, MAINTAIN ON tutorial.crm_customer, tutorial.billing_customer
	TO tutorial_admin;

SET SESSION AUTHORIZATION tutorial_login;
SET ROLE tutorial_admin;

SELECT 'crm' AS source, id, display_name AS name, '[' || email_address || ']' AS email
FROM tutorial.crm_customer
UNION ALL
SELECT 'erp', id, legal_name, '[' || email || ']'
FROM tutorial.billing_customer
ORDER BY source, id;
```

Expected:

```text
source|id|name|email
crm|1|Acme Ltd|[accounts@acme.example]
crm|2|Birch Studio|[hello@birch.example]
erp|101|ACME LIMITED|[ ACCOUNTS@ACME.EXAMPLE ]
erp|102|Cedar Works|[hello@cedar.example]
```

The brackets make surrounding spaces visible. The two Acme rows have different
source IDs and names. Their email addresses
agree after removing surrounding spaces and normalizing case. Birch and Cedar
have distinct email addresses, so we expect three customers from four records.

## 10–18 minutes: define the customer entity

We name the CRM source `crm` and the billing source `erp`.
Now, map each source's columns to shared `name` and `email` fields. Use the email
cleaner before exact matching, and prefer CRM values over billing values:

```sql
SELECT entity_name, desired_version, changed
FROM mdm.create(mdm.entity(
	name => 'customer',
	sources => ARRAY[
		mdm.source(
			name => 'crm',
			relation => 'tutorial.crm_customer'::regclass,
			source_id => ARRAY['id'],
			mode => 'tracked',
			fields => '{"name":"display_name","email":"email_address"}'::jsonb,
			row_changed_at => 'updated_at'),
		mdm.source(
			name => 'erp',
			relation => 'tutorial.billing_customer'::regclass,
			source_id => ARRAY['id'],
			mode => 'tracked',
			fields => '{"name":"legal_name","email":"email"}'::jsonb,
			row_changed_at => 'updated_at')
	],
	fields => ARRAY[
		mdm.field(name => 'name', type => 'text', cleaner => 'company_name'),
		mdm.field(name => 'email', type => 'text', cleaner => 'email')
	],
	matches => ARRAY[
		mdm.match(
			name => 'same_email',
			fields => ARRAY['email'],
			comparison => 'exact',
			strength => 'identity',
			evidence_group => 'email',
			candidate => '{"kind":"exact","field":"email"}'::jsonb)
	],
	golden_values => ARRAY[
		mdm.golden_value(
			field => 'name', policy => 'prefer_source',
			sources => ARRAY['crm', 'erp']),
		mdm.golden_value(
			field => 'email', policy => 'prefer_source',
			sources => ARRAY['crm', 'erp'])
	]
));
```

Expected:

```text
entity_name|desired_version|changed
customer|1|t
```

We have registered a definition. The first refresh will publish the results.
pg_trickle may also print notices and warnings about FULL refresh for this
graph. The lesson permits that refresh mode in the next step.

Pause to locate the five building blocks in the statement: `entity`, `source`,
`field`, `match`, and `golden_value`. The match rule decides which records belong
together. The golden-value policy decides which source value to publish.
Billing has the newer Acme row, but CRM comes first in our source preference.

In the current v0.12 implementation, `prefer_source` uses source-name order.
Its `sources` list filters eligible sources, but reversing that list does not
reverse priority. This example uses `crm` before `erp` so both orders agree.

Only fields listed in `golden_values` become customer value columns in the
output. We included both `name` and `email` so we can inspect them together.

## 18–23 minutes: publish and inspect the results

Refresh the entity. Select the stable parts of the response:

```sql
SELECT result->>'changed' AS changed,
	result->>'publication_revision' AS publication_revision,
	result->'source_boundary'->>'completeness' AS completeness
FROM (SELECT mdm.refresh('customer', 'ALLOW') AS result) AS refreshed;
```

Expected:

```text
changed|publication_revision|completeness
true|1|PROVEN
```

`ALLOW` permits a full graph refresh when needed. We use an explicit refresh
after source changes throughout this lesson.

The output tables now exist. Return to the administrator to grant read access,
then inspect them as our action role:

```sql
RESET ROLE;
RESET SESSION AUTHORIZATION;
GRANT USAGE ON SCHEMA mdm_out TO tutorial_admin;
GRANT SELECT ON mdm_out.customer, mdm_out.customer_members,
	mdm_out.customer_review TO tutorial_admin;
SET SESSION AUTHORIZATION tutorial_login;
SET ROLE tutorial_admin;

SELECT name, email, member_count, has_review
FROM mdm_out.customer
ORDER BY email;
```

Expected:

```text
name|email|member_count|has_review
Acme Ltd|accounts@acme.example|2|f
Birch Studio|hello@birch.example|1|f
Cedar Works|hello@cedar.example|1|f
```

Acme has two members and uses the CRM name. Cedar uses billing because Cedar
has no CRM record. Golden values preserve the selected source text. Cleaning
the email for matching did not rewrite the source tables.

Inspect the source membership behind each customer:

```sql
SELECT c.name, m.source_name, m.active
FROM mdm_out.customer AS c
JOIN mdm_out.customer_members AS m USING (mdm_id)
ORDER BY c.name, m.source_name;
```

Expected:

```text
name|source_name|active
Acme Ltd|crm|t
Acme Ltd|erp|t
Birch Studio|crm|t
Cedar Works|erp|t
```

Both Acme records point to the same `mdm_id`. This UUID identifies the resolved
customer and differs between fresh runs of the lesson.

Check the review output:

```sql
SELECT status, severity, reason_code
FROM mdm_out.customer_review
ORDER BY status, severity, reason_code;
```

Expect the column header with no rows:

```text
status|severity|reason_code
```

This fixture has no review issues. An empty review table does not prove that an
email match is appropriate for every real dataset. We deliberately use unique
customer email addresses in this lesson.

## 23–28 minutes: change a name and retain the identity

Save the current customer IDs in a temporary table before changing the source:

```sql
CREATE TEMP TABLE tutorial_ids AS
SELECT email, mdm_id FROM mdm_out.customer;
```

Return to the administrator, which owns our source tables. Change the CRM name
and its timestamp, then refresh as the action role:

```sql
RESET ROLE;
RESET SESSION AUTHORIZATION;
UPDATE tutorial.crm_customer
SET display_name = 'Acme Trading Ltd', updated_at = '2026-01-03 09:00:00+00'
WHERE id = 1;
SET SESSION AUTHORIZATION tutorial_login;
SET ROLE tutorial_admin;

SELECT result->>'changed' AS changed,
	result->>'publication_revision' AS publication_revision
FROM (SELECT mdm.refresh('customer', 'ALLOW') AS result) AS refreshed;
```

Expected:

```text
changed|publication_revision
true|2
```

Compare every current customer with its saved ID:

```sql
SELECT c.name, c.email, c.member_count,
	c.mdm_id = previous.mdm_id AS same_id
FROM mdm_out.customer AS c
JOIN tutorial_ids AS previous USING (email)
ORDER BY c.email;
```

Expected:

```text
name|email|member_count|same_id
Acme Trading Ltd|accounts@acme.example|2|t
Birch Studio|hello@birch.example|1|t
Cedar Works|hello@cedar.example|1|t
```

Acme's name changed while its `mdm_id` stayed the same. Our edit did not change
the email evidence or the records in the customer group.

## 28–30 minutes: verify a repeat refresh

Refresh once more without changing a source:

```sql
SELECT result->>'changed' AS changed,
	result->>'publication_revision' AS publication_revision
FROM (SELECT mdm.refresh('customer', 'ALLOW') AS result) AS refreshed;
```

Expected:

```text
changed|publication_revision
false|2
```

Check the complete customer result again:

```sql
SELECT name, email, member_count, has_review
FROM mdm_out.customer
ORDER BY email;
```

Expected:

```text
name|email|member_count|has_review
Acme Trading Ltd|accounts@acme.example|2|f
Birch Studio|hello@birch.example|1|f
Cedar Works|hello@cedar.example|1|f
```

You have defined two sources, matched their duplicate customer, published golden
values, and refreshed a source change while retaining customer identities.
The three output tables give you customer values, source membership, and review
issues. You can query them with ordinary SQL.

For your next exercise, use the same pattern with a second entity in this
disposable database. Keep fuzzy matching, manual stewardship, and production
tuning outside this first lesson. The [project README](README.md) links to the
design documents when you are ready to explore those topics.

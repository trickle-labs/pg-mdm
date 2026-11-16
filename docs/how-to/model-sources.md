# Model your source tables and matching rules

[Documentation home](../README.md) · How-to guide

Use this guide after the first tutorial to create synthetic fixtures that represent your domain and test a definition against them. The goal is a set of identity rules you can explain and a candidate set you can evaluate completely. You need an isolated installation, an execution role with the required privileges, and synthetic examples of both duplicates and lookalike records that must stay separate. The current [demo preview is not qualified for real workloads](../reference/compatibility.md); do not connect actual source systems while rehearsing this workflow.

## Choose a durable source key

Choose the columns that identify a record within its source system, rather than a value you hope will identify the resolved entity. A CRM primary key is a useful source key; an email address that users can edit or share usually is not. Each source must have an immediate, valid unique index matching the declared key columns in order, without expressions or a partial predicate. The key columns must be non-null, or the index must use `NULLS NOT DISTINCT`. Ordinary and partitioned non-temporary PostgreSQL tables are supported source shapes; the current changed-key optimization has a narrower qualification boundary described in [Operations](operate.md).

The simplest source declaration maps physical columns to shared logical fields:

```sql
SELECT mdm.source(
    name => 'crm',
    relation => 'public.crm_customer'::regclass,
    source_id => ARRAY['id'],
    mode => 'tracked',
    fields => '{"name":"display_name","email":"email_address"}'::jsonb,
    row_changed_at => 'updated_at'
);
```

This constructor returns JSON; it becomes part of the array passed to `mdm.entity()`. The relation and columns must already exist when you prepare the definition. As the installation administrator, grant the execution role `USAGE` on the source schema and `SELECT, MAINTAIN` on each source table. A source's logical name, relation identity, key contract, and retained source history are consequential: treat renames, key changes, and table replacement as migrations rather than cosmetic edits.

## Represent deletion explicitly

Use `mode => 'tracked'` when the table's present rows define its active records. For a source that retains deleted rows, use `mode => 'soft_delete'` and a supported predicate such as `'{"column":"deleted_at","kind":"is_not_null"}'::jsonb` or `'{"column":"is_deleted","kind":"is_true"}'::jsonb`. The referenced column must have the appropriate physical type. A deletion changes active membership while retaining source identity and historical evidence; do not repurpose an old source key for a different real-world record and expect it to become an unrelated history.

## Normalize for comparison, preserve values for readers

Define each shared field with its logical type and a cleaner. `email` handles case and surrounding whitespace for matching, while `company_name` provides conservative text normalization for company names. A cleaner establishes comparison values; it does not update the source table or automatically replace the raw value selected for a golden output. Decide separately which fields need to be published, because only fields listed in `golden_values` become golden-value columns.

```sql
SELECT mdm.field(name => 'email', type => 'text', cleaner => 'email');
SELECT mdm.field(name => 'name', type => 'text', cleaner => 'company_name');
```

Use [Definition reference](../reference/definitions.md#cleaners) to check supported cleaners, types, options, and invalid-value behavior. Test representative edge cases, such as shared addresses, punctuation, empty strings, and malformed identifiers, before treating normalized agreement as identity evidence. The `display` option defaults to `masked`; it is an explanation presentation choice, not a substitute for output-table access control.

## Start with a rule you can defend

An exact identity rule is a good first experiment only when exact agreement is meaningful for your domain. In the tutorial, a carefully selected email address identifies the customer. In a household, supplier, or shared-service dataset, the same assumption may join unrelated records. Write down the business meaning before writing the SQL, and keep examples of both positive and negative cases beside your definition.

```sql
SELECT mdm.match(
    name => 'same_email',
    fields => ARRAY['email'],
    comparison => 'exact',
    strength => 'identity',
    evidence_group => 'email',
    candidate => '{"kind":"exact","field":"email"}'::jsonb
);
```

Candidate predicates determine which pairs are eligible for comparison. They are part of the rule's meaning: a fuzzy comparator cannot find a pair excluded by its candidate predicate. For example, the following rule compares company names only within a shared four-character prefix. It demonstrates the supported syntax; choose the prefix and threshold using your own fixtures rather than adopting these values as a universal matching policy.

```sql
SELECT mdm.match(
    name => 'similar_company_name',
    fields => ARRAY['name'],
    comparison => 'normalized_levenshtein',
    strength => 'strong',
    evidence_group => 'company_name',
    threshold => 9000,
    candidate => '{"kind":"prefix","field":"name","length":4}'::jsonb
);
```

Scores use the integer range 0–10000. Strong and non-exact identity rules require an explicit candidate; supporting rules do not own a candidate channel. Evidence groups distinguish independent evidence, so two rules derived from the same underlying field cannot be presented as two independent reasons to merge. See [Entity resolution](../explanation/entity-resolution.md) for why a locally plausible pair does not always justify growing a larger component.

## Choose golden policies deliberately

Use `latest` when a trustworthy change timestamp should decide the value, `most_common` when agreement across eligible records is meaningful, `first_non_null` when a deterministic available value is sufficient, or `prefer_source` when source ordering is appropriate. In the current implementation, definitions are canonicalized by source name and that order becomes source priority. The `sources` array in a golden-value rule restricts eligible sources; reversing the array does not reverse priority. If only CRM should provide a particular field, restrict the eligible list to `ARRAY['crm']` and decide how you want to handle entities without a usable CRM value.

Assemble the chosen constructors into `mdm.entity()`, then use [Preview and publish a definition change](change-a-definition.md) to register, inspect, and activate the definition. Keep candidate and resolver limits explicit when your data approaches their defaults. A limit error leaves the old publication intact; it is a signal to examine the data and candidate design before raising the bound.

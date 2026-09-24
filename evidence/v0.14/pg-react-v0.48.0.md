# pg-react v0.48.0 joint qualification

**Result:** passed on 2026-09-24 in the disposable `pg-react-mdm-m2-clean` container.

## Source and package identity

| Item | Identity |
| --- | --- |
| pg-react base commit | `991bf9fd1d3de6102077afaeb1c9f64a11c1681c`; the qualification changes were in the uncommitted worktree |
| pg-mdm commit | `5366a820e0db6a7d65567a2a6e38e7276ce8e97f` |
| pg-react extension | `0.46.0` |
| pg-mdm extension | `0.14.0` |
| pg_trickle extension | `0.108.0` |
| pg-mdm base image | `pg_mdm@sha256:2b3da6d0fbbad930bab45aca7b12d691e9e8801548f06135c0877ed657ee7e9d` |
| Joint runtime image used for SQL qualification | `pg-react-mdm@sha256:72f4bbb5d3f1bd55512d4ca937cf15932e23fb0a83025289461bd6ed0bfa0705` |
| Joint image rebuilt with the restore helper file | `pg-react-mdm@sha256:dd194d0e4739f7e4c148a51b4a8536bd9271fc57806087d433ba9f4d32747fef` |
| M2 policy package | `v0.48-live-policy`, digest `b60bff5c255647790476fe398e329da39ff48fa07dce32aa9842c0ae5cbd7db6` |
| pg-mdm shared library | `64d556d6afdd209f24115ada3eab36bff2fa2f5b0ef127ca2f37b7511df9488d` |
| pg-mdm 0.14.0 install SQL | `b65e9bcb9b9f2b4ed35fd9477b6b948ced72ab0e9126f05f347ee92de53f92d4` |
| pg-mdm configure helper SQL | `f62b2056b1faad1f87049e77cc61d7ceda02218292cb6bc8b2dffb9e0636ad37` |

The restore run used the helper SQL copied from the pg-mdm image. The rebuilt joint image contains the same file at `tests/integrations/pg-mdm/pg-mdm-configure-helper.sql`.

## Roles exercised

The setup connection used `postgres`. The worker test connected as `mdm_m2_runner` and ran with `SET ROLE pgreact_mdm_worker`. The worker was `NOLOGIN`, `NOSUPERUSER`, `NOBYPASSRLS`, and `NOINHERIT`. The entity roles were `mdm_legacy_administrator` and `mdm_administrator`. The MDM helper role was `mdm_helper_owner`.

## Commands and results

The SQL tests ran against database `foundation` on port `55432`.

```sh
rtk psql -h 127.0.0.1 -p 55432 -U postgres -d foundation -v ON_ERROR_STOP=1 -f tests/integrations/pg-mdm/v0.48-codec.sql
rtk psql -h 127.0.0.1 -p 55432 -U postgres -d foundation -v ON_ERROR_STOP=1 -f tests/integrations/pg-mdm/v0.47-live-setup.sql
rtk psql -h 127.0.0.1 -p 55432 -U postgres -d foundation -v ON_ERROR_STOP=1 -f tests/integrations/pg-mdm/v0.47-live.sql
rtk psql -h 127.0.0.1 -p 55432 -U postgres -d foundation -v ON_ERROR_STOP=1 -f tests/integrations/pg-mdm/v0.48-live.sql
rtk psql -h 127.0.0.1 -p 55432 -U postgres -d foundation -v ON_ERROR_STOP=1 -f tests/integrations/pg-mdm/v0.48-security.sql
PGHOST=127.0.0.1 PGPORT=55432 PGUSER=postgres MDM_HELPER_CONFIG=/private/tmp/mdm-m2-configure_helper.sql rtk sh tests/integrations/pg-mdm/v0.48-restore.sh
```

The commands returned zero. Their summary output was:

```text
v0.48 canonical digest, request-key, body, and retry checks: PASS
v0.47 live MDM policy package passed
v0.48 actual worker actions, limits, receipts, and observation stability passed
v0.48 actual worker privileges and retry passed
v0.48 logical restore and OID reconciliation passed
```

The restore check confirmed durable request, attempt, and receipt correlation. It rejected work before entity rebind and runtime reconciliation. The MDM runtime row was absent in the restored database. React's saved runtime row kept the old database and worker OIDs. Reconciliation then recorded the current identities.

This M2 check does not recompile MDM graphs. The upstream pg-mdm restore test covers that path. A joint-image attempt to recompile hit a reset `pgtrickle.pgt_stream_tables_pgt_id_seq` and a duplicate `pgt_id`.

The updated image built with:

```sh
rtk docker build --quiet -f tests/integrations/pg-mdm/Dockerfile.v0.48 -t pg-react-mdm:0.48-m2 .
```

The build returned `sha256:dd194d0e4739f7e4c148a51b4a8536bd9271fc57806087d433ba9f4d32747fef`. A container check confirmed that the packaged configure helper file exists and matches the file used by the restore command.

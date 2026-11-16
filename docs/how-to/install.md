# Install pg_mdm

[Documentation home](../README.md) · How-to guide

This guide gets you to an isolated PostgreSQL server with the pg_mdm extension files installed. The easiest reproducible route for learning is the repository's Docker image, which builds pg_mdm and its pinned pg_trickle dependency together. A native route is also available for a dedicated test server. Both routes target the **0.15.0 demo preview for synthetic/demo data only**. Do not connect this preview to a real workload or enable live controls; its [release qualification gates remain blocked](https://github.com/trickle-labs/pg-mdm/issues/28). Read [Compatibility and status](../reference/compatibility.md) before proceeding.

## Build a local learning environment

You need Git, Docker with BuildKit support, access to GitHub and the build dependencies, and enough time for a cold Rust build. The image targets `linux/amd64`; Docker must support that platform if your machine uses another architecture. The repository pins the PostgreSQL image, Rust and pgrx versions, pg_trickle source revision, and source-archive checksum in [tests/Dockerfile.e2e](../../tests/Dockerfile.e2e). Do not substitute an arbitrary pg_trickle package: the integration depends on specific graph and output-delta capabilities.

Clone the repository and select the demo preview tag so that the source, scripts, and lesson agree:

```sh
git clone https://github.com/trickle-labs/pg-mdm.git
cd pg-mdm
git checkout v0.15.0
scripts/build_e2e_image.sh
```

The build script requires a clean checkout so its image label identifies the code being built. If you are reviewing a documentation branch, stay on that branch instead of checking out the tag, and commit the reviewed changes before using the script. The resulting image is named `pg_mdm:0.15.0-e2e`. It already configures `shared_preload_libraries = 'pg_trickle,pg_mdm'`; it also enables the producer's output-delta qualification setting used by this learning and test environment.

Start a disposable instance with a repository mount so `psql` can include the helper setup script:

```sh
docker run --detach --name pg-mdm-tutorial \
  -e POSTGRES_PASSWORD=local-tutorial-only \
  -p 127.0.0.1:55432:5432 \
  --mount type=bind,source="$(pwd)",target=/work,readonly \
  --workdir /work \
  pg_mdm:0.15.0-e2e

docker logs pg-mdm-tutorial
docker exec pg-mdm-tutorial pg_isready -h 127.0.0.1 -U postgres
```

Wait until the logs show that initialization has completed and the final server is ready, then repeat `pg_isready` if necessary. The password is only for this loopback-bound learning instance. Continue with [Your first customer golden records](../tutorials/first-entity.md), which creates a database, installs the extensions in that database, configures the helper owner, and grants the learning role the actions it needs. You do not need a host installation of `psql` for that route.

When you have finished both tutorials, remove the disposable container and its anonymous database volume with `docker rm -fv pg-mdm-tutorial`. This deletes the tutorial database. Keep the container while working through the second lesson, which builds on the first lesson's data.

## Install into an existing PostgreSQL 18 server

Use this route only on a dedicated synthetic-data test server where you can install extension files and restart PostgreSQL. You need PostgreSQL 18 development headers and `pg_config`, the Rust and pgrx versions in [DEPENDENCIES.md](../../DEPENDENCIES.md), and a pg_trickle package built from the matching pinned source. The [demo release](https://github.com/trickle-labs/pg-mdm/releases/tag/v0.15.0) includes a Linux amd64 package and checksum; those assets are available, but their availability does not complete the release qualification gates or install the producer dependency. Check paths, platform, checksum, and PostgreSQL major version before copying any files.

For a source build, use the repository's [Dockerfile](../../tests/Dockerfile.e2e) as the complete reproducible build recipe. On an already configured native Rust/pgrx toolchain, the adapter packaging step is:

```sh
cargo pgrx package --pg-config "$(command -v pg_config)"
sudo cp target/release/pg_mdm-pg18/*/lib/postgresql/18/lib/pg_mdm.* "$(pg_config --pkglibdir)/"
sudo cp target/release/pg_mdm-pg18/*/share/postgresql/18/extension/pg_mdm* "$(pg_config --sharedir)/extension/"
```

Install the corresponding pg_trickle files first. Add both extensions to the server's existing preload list, preserving any other libraries already configured, and restart PostgreSQL. For this preview's policy automation, pg_mdm's preload is required; a reload alone is insufficient.

```conf
shared_preload_libraries = 'pg_trickle,pg_mdm'
```

Connect to the target database as its installation administrator and check the versions available before creating the extensions:

```sql
SELECT name, default_version, installed_version
FROM pg_available_extensions
WHERE name IN ('pg_mdm', 'pg_trickle')
ORDER BY name;

CREATE EXTENSION pg_trickle;
CREATE EXTENSION pg_mdm;
CREATE ROLE mdm_helper_owner NOLOGIN NOSUPERUSER NOBYPASSRLS;
```

The helper role must have no members and no memberships in other roles. Run the repository script from a terminal as the database administrator, supplying your database connection normally:

```sh
psql -X --set=ON_ERROR_STOP=1 \
  --set=helper_owner=mdm_helper_owner \
  --file=sql/configure_helper.sql my_demo_database
```

## Create an action role and verify the installation

The installation administrator owns the setup process; application work uses a restricted login and a selected action role. This separation is part of the API contract. pg_mdm checks both `session_user` and `current_user`, so calling it from a superuser session after only `SET ROLE` is not sufficient. Create the roles and grants as the administrator:

```sql
CREATE ROLE app_mdm_admin NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE app_login LOGIN NOSUPERUSER NOBYPASSRLS;
GRANT app_mdm_admin TO app_login WITH SET TRUE, INHERIT FALSE;
GRANT USAGE ON SCHEMA mdm, mdm_admin TO app_mdm_admin;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA mdm TO app_mdm_admin;
GRANT EXECUTE ON FUNCTION mdm_admin.verify_installation() TO app_mdm_admin;
GRANT USAGE ON SCHEMA pgtrickle TO app_mdm_admin;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA pgtrickle TO app_mdm_admin;
```

Configure authentication for `app_login` using your normal PostgreSQL procedure, then connect as that login. Select the action role and check the installation:

```sql
SET ROLE app_mdm_admin;
SELECT session_user, current_user;
SELECT mdm_admin.verify_installation()::uuid IS NOT NULL AS installation_recorded;
```

The identities should be `app_login` and `app_mdm_admin`, and `installation_recorded` should be `true`. The function returns an **operation UUID as text** on success; `MDM_OK` is the success code recorded with the operation, not the returned string. A failure raises a stable `MDM_*` result code with a message. Use [Troubleshooting](troubleshoot.md) if the ownership, caller, or producer-capability checks fail.

Before defining an entity, grant its execution role schema `USAGE` and source-table `SELECT, MAINTAIN` privileges. Grant consumers `SELECT` on the published output tables after the first refresh creates them. Rerun `configure_helper.sql` and review action grants after an extension upgrade or logical restore. The [security explanation](../explanation/security.md) describes what the helper owns and why it must remain separate from application roles.

**Next:** [Build your first entity](../tutorials/first-entity.md), or [connect your own source tables](model-sources.md).

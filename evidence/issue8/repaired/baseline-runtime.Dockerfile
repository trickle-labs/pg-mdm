FROM pg_mdm:issue12-final
COPY --from=pg_mdm:issue8-baseline-builder /package/usr/lib/postgresql/18/lib/ /usr/lib/postgresql/18/lib/
COPY --from=pg_mdm:issue8-baseline-builder /package/usr/share/postgresql/18/extension/ /usr/share/postgresql/18/extension/
COPY --from=pg_mdm:issue8-baseline-builder /tmp/pg_mdm-package-manifest.sha256 /evidence/pg_mdm-package-manifest.sha256
LABEL pg_mdm.source_revision=9a07830d062b31c108880071c27124e2f09c446a

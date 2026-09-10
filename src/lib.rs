#![deny(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod api;
pub mod catalog;
pub mod cleaners;
pub mod definition;
pub mod error;
pub mod graph_spec;
pub mod integration;
pub mod normalization;
pub mod presets;
pub mod schema;
pub mod source_record;
pub mod version;

::pgrx::pg_module_magic!();

fn raise(error: error::MdmError) -> ! {
    pgrx::error!("{}: {}", error.code(), error)
}

#[cfg(feature = "pg_test")]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    #[pg_test]
    fn baseline_capabilities_are_reported_as_disabled() {
        let rows = Spi::get_one::<i64>(
            "SELECT count(*) FROM mdm_internal.integration_capabilities() WHERE major_version = 1 AND minor_version = 0 AND NOT enabled",
        )
        .expect("capability query must run");
        assert_eq!(rows, Some(2));
    }
}

#[cfg(feature = "pg_test")]
#[allow(dead_code)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}

    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec!["shared_preload_libraries = 'pg_trickle'"]
    }
}

#![deny(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod api;
pub mod candidate;
pub mod catalog;
pub mod cleaners;
pub mod comparators;
pub mod constraint;
pub mod decision;
pub mod definition;
pub mod error;
pub mod evidence;
pub mod golden;
pub mod graph_spec;
pub mod identity;
pub mod integration;
pub mod normalization;
pub mod output;
pub mod pair;
pub mod presets;
pub mod resolver;
pub mod review;
pub mod schema;
pub mod semantics;
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
    fn graph_v1_capabilities_are_enabled() {
        let rows = Spi::get_one::<i64>(
            "SELECT count(*) FROM mdm_internal.integration_capabilities() WHERE major_version = 1 AND minor_version = 0 AND enabled",
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

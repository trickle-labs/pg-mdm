mod error {
    pub use pg_mdm::error::MdmError;
}

mod normalization {
    pub use pg_mdm::normalization::NormalizedState;
}

#[path = "../src/golden.rs"]
mod golden;

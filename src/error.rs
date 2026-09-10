use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MdmError {
    #[error("required pg_trickle capability is missing: {0}")]
    CapabilityMissing(&'static str),
    #[error("unsupported {capability} major version {major}; expected 1")]
    CapabilityVersion { capability: String, major: i16 },
    #[error("invalid pg_trickle capability response: {0}")]
    CapabilityInvalid(String),
    #[error("external_graph_refresh 1.x is disabled by pg_trickle")]
    GraphCapabilityDisabled,
    #[error("helper ownership is unsafe: {0}")]
    HelperOwnerUnsafe(String),
    #[error("caller is not authorized: {0}")]
    Unauthorized(String),
    #[error("operation state is invalid: {0}")]
    OperationState(String),
    #[error("PostgreSQL SPI failed: {0}")]
    Spi(String),
    #[error("invalid MDM definition: {0}")]
    DefinitionInvalid(String),
    #[error("invalid source contract: {0}")]
    SourceInvalid(String),
    #[error("output name is already reserved: {0}")]
    OutputNameConflict(String),
    #[error("definition version conflict: {0}")]
    VersionConflict(String),
    #[error("cleaner {cleaner} version {version} is not supported")]
    CleanerVersion { cleaner: String, version: i32 },
    #[error("cleaner {0} is invalid: {1}")]
    CleanerInvalid(String, String),
    #[error("cleaner execution error: {0}")]
    CleanerExecution(String),
    #[error("source record key is invalid: {0}")]
    SourceRecord(String),
}

impl MdmError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::CapabilityMissing(_) => "MDM_PGT_CAPABILITY_MISSING",
            Self::CapabilityVersion { .. } => "MDM_PGT_CAPABILITY_VERSION",
            Self::CapabilityInvalid(_) => "MDM_PGT_CAPABILITY_INVALID",
            Self::GraphCapabilityDisabled => "MDM_PGT_CAPABILITY_DISABLED",
            Self::HelperOwnerUnsafe(_) => "MDM_HELPER_OWNER_UNSAFE",
            Self::Unauthorized(_) => "MDM_UNAUTHORIZED",
            Self::OperationState(_) => "MDM_OPERATION_STATE",
            Self::Spi(_) => "MDM_INTERNAL",
            Self::DefinitionInvalid(_) => "MDM_DEFINITION_INVALID",
            Self::SourceInvalid(_) => "MDM_SOURCE_INVALID",
            Self::OutputNameConflict(_) => "MDM_OUTPUT_NAME_CONFLICT",
            Self::VersionConflict(_) => "MDM_VERSION_CONFLICT",
            Self::CleanerVersion { .. } => "MDM_CLEANER_VERSION",
            Self::CleanerInvalid(..) => "MDM_CLEANER_INVALID",
            Self::CleanerExecution(_) => "MDM_CLEANER_ERROR",
            Self::SourceRecord(_) => "MDM_SOURCE_RECORD_INVALID",
        }
    }
}

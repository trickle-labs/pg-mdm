use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum MdmError {
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
}

impl MdmError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::CapabilityMissing(_) => "MDM_PGT_CAPABILITY_MISSING",
            Self::CapabilityVersion { .. } => "MDM_PGT_CAPABILITY_VERSION",
            Self::CapabilityInvalid(_) => "MDM_PGT_CAPABILITY_INVALID",
            Self::GraphCapabilityDisabled => "MDM_PGT_CAPABILITY_DISABLED",
            Self::HelperOwnerUnsafe(_) => "MDM_HELPER_OWNER_UNSAFE",
            Self::Unauthorized(_) => "MDM_UNAUTHORIZED",
            Self::OperationState(_) => "MDM_OPERATION_STATE",
            Self::Spi(_) => "MDM_INTERNAL",
        }
    }
}

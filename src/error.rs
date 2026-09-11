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
    #[error("invalid graph artifact: {0}")]
    GraphArtifact(String),
    #[error("graph installation failed: {0}")]
    GraphInstallation(String),
    #[error("graph contract is invalid: {0}")]
    GraphContract(String),
    #[error("graph binding is invalid: {0}")]
    GraphBinding(String),
    #[error("graph lifecycle failed: {0}")]
    GraphLifecycle(String),
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
    #[error(
        "candidate block for channel {channel} has {cardinality} records; limit is {limit}; raise max_block_records or split the definition"
    )]
    CandidateBlockLimit {
        channel: String,
        cardinality: usize,
        limit: usize,
    },
    #[error(
        "candidate set has at least {candidate_pairs} pairs; limit is {limit}; raise max_candidate_pairs or split the definition"
    )]
    CandidateTotalLimit {
        candidate_pairs: usize,
        limit: usize,
    },
    #[error("candidate pair count overflowed while evaluating channel {channel}")]
    CandidateCountOverflow { channel: String },
    #[error("invalid candidate plan: {0}")]
    CandidateInvalid(String),
    #[error("invalid evidence: {0}")]
    EvidenceInvalid(String),
    #[error("invalid comparator: {0}")]
    ComparatorInvalid(String),
    #[error("comparator work limit exceeded: {work} > {limit}")]
    ComparatorWorkLimit { work: usize, limit: usize },
    #[error("decision is invalid: {0}")]
    DecisionInvalid(String),
    #[error("decision version conflict: {0}")]
    DecisionVersionConflict(String),
    #[error("manual decision contradiction: {0}")]
    DecisionContradiction(String),
    #[error("decision check limit exceeded: {checked} > {limit}")]
    DecisionCheckLimit { checked: usize, limit: usize },
    #[error("resolver limit exceeded for {resource}: {observed} > {limit}")]
    ResolverLimit {
        resource: &'static str,
        observed: usize,
        limit: usize,
    },
    #[error("invalid resolver input: {0}")]
    ResolverInvalid(String),
    #[error("resolver invariant failed: {0}")]
    ResolverInvariant(String),
    #[error("invalid identity history: {0}")]
    IdentityInvalid(String),
    #[error("identity history exceeds the {resource} limit: {observed} > {limit}")]
    IdentityLimit {
        resource: &'static str,
        observed: usize,
        limit: usize,
    },
    #[error("invalid golden value: {0}")]
    GoldenInvalid(String),
    #[error("golden override conflict: {0}")]
    GoldenConflict(String),
    #[error("invalid review history: {0}")]
    ReviewInvalid(String),
    #[error("invalid output schema: {0}")]
    OutputInvalid(String),
    #[error("explanation is not retained: {0}")]
    ExplanationNotRetained(String),
    #[error("invalid explanation request: {0}")]
    ExplanationInvalid(String),
}

impl MdmError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::CapabilityMissing(_) => "MDM_PGT_CAPABILITY_MISSING",
            Self::CapabilityVersion { .. } => "MDM_PGT_CAPABILITY_VERSION",
            Self::CapabilityInvalid(_) => "MDM_PGT_CAPABILITY_INVALID",
            Self::GraphCapabilityDisabled => "MDM_PGT_CAPABILITY_DISABLED",
            Self::GraphArtifact(_) => "MDM_GRAPH_ARTIFACT",
            Self::GraphInstallation(_) => "MDM_GRAPH_INSTALLATION",
            Self::GraphContract(_) => "MDM_GRAPH_CONTRACT",
            Self::GraphBinding(_) => "MDM_GRAPH_BINDING",
            Self::GraphLifecycle(_) => "MDM_GRAPH_LIFECYCLE",
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
            Self::CandidateBlockLimit { .. } => "MDM_CANDIDATE_BLOCK_LIMIT",
            Self::CandidateTotalLimit { .. } | Self::CandidateCountOverflow { .. } => {
                "MDM_CANDIDATE_TOTAL_LIMIT"
            }
            Self::CandidateInvalid(_) => "MDM_CANDIDATE_INVALID",
            Self::EvidenceInvalid(_) => "MDM_EVIDENCE_INVALID",
            Self::ComparatorInvalid(_) => "MDM_COMPARATOR_INVALID",
            Self::ComparatorWorkLimit { .. } => "MDM_COMPARATOR_LIMIT",
            Self::DecisionInvalid(_) => "MDM_DECISION_INVALID",
            Self::DecisionVersionConflict(_) => "MDM_DECISION_VERSION_CONFLICT",
            Self::DecisionContradiction(_) => "MDM_DECISION_CONTRADICTION",
            Self::DecisionCheckLimit { .. } => "MDM_DECISION_CHECK_LIMIT",
            Self::ResolverLimit { .. } => "MDM_RESOLVER_LIMIT",
            Self::ResolverInvalid(_) => "MDM_RESOLVER_INVALID",
            Self::ResolverInvariant(_) => "MDM_RESOLVER_INVARIANT",
            Self::IdentityInvalid(_) => "MDM_IDENTITY_INVALID",
            Self::IdentityLimit { .. } => "MDM_IDENTITY_LIMIT",
            Self::GoldenInvalid(_) => "MDM_GOLDEN_INVALID",
            Self::GoldenConflict(_) => "MDM_GOLDEN_OVERRIDE_CONFLICT",
            Self::ReviewInvalid(_) => "MDM_REVIEW_INVALID",
            Self::OutputInvalid(_) => "MDM_OUTPUT_INVALID",
            Self::ExplanationNotRetained(_) => "MDM_EXPLANATION_NOT_RETAINED",
            Self::ExplanationInvalid(_) => "MDM_EXPLANATION_INVALID",
        }
    }
}

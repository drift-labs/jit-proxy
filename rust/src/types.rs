use drift_sdk::types::SdkError;
use thiserror::Error;

pub type JitResult<T> = Result<T, JitError>;

#[derive(Debug, Error)]
pub enum JitError {
    #[error("{0}")]
    Drift(String),
    #[error("{0}")]
    Sdk(String),
}

impl From<drift::error::ErrorCode> for JitError {
    fn from(error: drift::error::ErrorCode) -> Self {
        JitError::Drift(error.to_string())
    }
}

impl From<SdkError> for JitError {
    fn from(error: SdkError) -> Self {
        JitError::Sdk(error.to_string())
    }
}

#[derive(Clone, Copy)]
pub struct ComputeBudgetParams {
    microlamports_per_cu: u64,
    cu_limit: u32,
}

impl ComputeBudgetParams {
    pub fn new(microlamports_per_cu: u64, cu_limit: u32) -> Self {
        Self {
            microlamports_per_cu,
            cu_limit,
        }
    }

    pub fn microlamports_per_cu(&self) -> u64 {
        self.microlamports_per_cu
    }

    pub fn cu_limit(&self) -> u32 {
        self.cu_limit
    }
}

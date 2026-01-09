use crate::ln_verification::LightningVerificationEntity;

/// Response type for awaiting verification with timeout status
#[derive(Debug, Clone)]
pub enum VerificationResponse {
    /// Verification completed successfully (within timeout or already finalised)
    Success(LightningVerificationEntity),
    /// Verification exists but timed out waiting for payment
    TimedOut,
}

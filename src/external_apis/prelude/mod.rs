pub mod mock_prelude_api;
mod prelude_api;

pub use mock_prelude_api::MockSmsVerificationProviderApi;
pub use prelude_api::{
    PreludeAPI, PreludeCheckCodeResponse, PreludeVerificationResponse, SmsVerificationProviderApi,
};

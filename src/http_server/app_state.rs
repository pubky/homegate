use crate::external_apis::{HomeserverAdminApiTrait, SmsVerificationProviderApi};
use crate::persistence::db::Db;
use crate::sms_verification::SmsVerificationService;

#[derive(Clone, Debug)]
pub struct AppState<
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
> {
    pub db: Db,
    pub sms_verification_service: SmsVerificationService<T, S>,
}

use crate::services::codearts_accounts::{self, Account, LoginStart};
use crate::{
    models::model::AccountProvider, services::account_model_sources::finish_account_deletion,
};

#[tauri::command]
pub async fn list_codearts_accounts() -> Result<Vec<Account>, String> {
    codearts_accounts::list_accounts().await
}
#[tauri::command]
pub fn list_saved_codearts_accounts() -> Result<Vec<Account>, String> {
    codearts_accounts::list_saved_accounts()
}
#[tauri::command]
pub async fn start_codearts_login() -> Result<LoginStart, String> {
    codearts_accounts::start_login().await
}
#[tauri::command]
pub async fn poll_codearts_login(login_id: String) -> Result<Option<Account>, String> {
    codearts_accounts::poll_login(&login_id).await
}
#[tauri::command]
pub fn cancel_codearts_login(login_id: String) -> Result<(), String> {
    codearts_accounts::cancel_login(&login_id)
}
#[tauri::command]
pub async fn refresh_codearts_account(account_id: String) -> Result<Account, String> {
    codearts_accounts::refresh_account(&account_id).await
}
#[tauri::command]
pub async fn switch_codearts_account(account_id: String) -> Result<Account, String> {
    codearts_accounts::switch_account(&account_id).await
}
#[tauri::command]
pub async fn delete_codearts_account(account_id: String) -> Result<(), String> {
    finish_account_deletion(
        AccountProvider::CodeArts,
        &account_id,
        codearts_accounts::delete_account(&account_id).await,
    )
}
#[tauri::command]
pub async fn claim_codearts_credits(account_id: String) -> Result<Account, String> {
    codearts_accounts::claim_credits(&account_id).await
}

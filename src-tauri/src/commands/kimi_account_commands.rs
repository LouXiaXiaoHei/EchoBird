use crate::services::kimi_accounts::{self, Account, LoginStart, Region};
use crate::{
    models::model::AccountProvider, services::account_model_sources::finish_account_deletion,
};

#[tauri::command]
pub async fn list_kimi_accounts() -> Result<Vec<Account>, String> {
    kimi_accounts::list().await
}
#[tauri::command]
pub async fn start_kimi_login(region: Region) -> Result<LoginStart, String> {
    kimi_accounts::start_login(region).await
}
#[tauri::command]
pub async fn poll_kimi_login(login_id: String) -> Result<Option<Account>, String> {
    kimi_accounts::poll_login(&login_id).await
}
#[tauri::command]
pub fn cancel_kimi_login(login_id: String) -> Result<(), String> {
    kimi_accounts::cancel_login(&login_id)
}
#[tauri::command]
pub async fn refresh_kimi_account(account_id: String) -> Result<Account, String> {
    kimi_accounts::refresh(&account_id).await
}
#[tauri::command]
pub async fn switch_kimi_account(account_id: String) -> Result<Account, String> {
    kimi_accounts::switch(&account_id).await
}
#[tauri::command]
pub async fn delete_kimi_account(account_id: String) -> Result<(), String> {
    finish_account_deletion(
        AccountProvider::Kimi,
        &account_id,
        kimi_accounts::delete(&account_id).await,
    )
}

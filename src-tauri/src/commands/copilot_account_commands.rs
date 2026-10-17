use crate::services::copilot_accounts::{self, Account, LoginStart};
use crate::{
    models::model::AccountProvider, services::account_model_sources::finish_account_deletion,
};

#[tauri::command]
pub async fn list_copilot_accounts(tool_id: Option<String>) -> Result<Vec<Account>, String> {
    copilot_accounts::list(tool_id.as_deref()).await
}

#[tauri::command]
pub async fn import_copilot_accounts() -> Result<Vec<Account>, String> {
    copilot_accounts::import_native_accounts().await
}

#[tauri::command]
pub async fn start_copilot_login(tool_id: String) -> Result<LoginStart, String> {
    copilot_accounts::start_login(&tool_id).await
}

#[tauri::command]
pub async fn poll_copilot_login(login_id: String) -> Result<Option<Account>, String> {
    copilot_accounts::poll_login(&login_id).await
}

#[tauri::command]
pub fn cancel_copilot_login(login_id: String) -> Result<(), String> {
    copilot_accounts::cancel_login(&login_id)
}

#[tauri::command]
pub async fn switch_copilot_account(
    tool_id: String,
    account_id: String,
) -> Result<Account, String> {
    copilot_accounts::switch(&tool_id, &account_id).await
}

#[tauri::command]
pub async fn refresh_copilot_account(account_id: String) -> Result<Account, String> {
    copilot_accounts::refresh(&account_id).await
}

#[tauri::command]
pub async fn delete_copilot_account(account_id: String) -> Result<(), String> {
    finish_account_deletion(
        AccountProvider::Copilot,
        &account_id,
        copilot_accounts::delete(&account_id).await,
    )
}

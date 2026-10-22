use crate::services::qwenwork_accounts::{self, Account, Edition, LoginStart};

#[tauri::command]
pub async fn list_qwenwork_accounts(edition: Edition) -> Result<Vec<Account>, String> {
    qwenwork_accounts::list(edition).await
}
#[tauri::command]
pub async fn start_qwenwork_login(edition: Edition) -> Result<LoginStart, String> {
    qwenwork_accounts::start_login(edition).await
}
#[tauri::command]
pub async fn poll_qwenwork_login(login_id: String) -> Result<Option<Account>, String> {
    qwenwork_accounts::poll_login(&login_id).await
}
#[tauri::command]
pub async fn cancel_qwenwork_login(login_id: String) -> Result<(), String> {
    qwenwork_accounts::cancel_login(&login_id)
}
#[tauri::command]
pub async fn refresh_qwenwork_account_quota(
    edition: Edition,
    account_id: String,
) -> Result<Account, String> {
    qwenwork_accounts::refresh(edition, &account_id).await
}
#[tauri::command]
pub async fn delete_qwenwork_account(edition: Edition, account_id: String) -> Result<(), String> {
    qwenwork_accounts::delete(edition, &account_id).await
}
#[tauri::command]
pub async fn switch_qwenwork_account(
    edition: Edition,
    account_id: String,
) -> Result<Account, String> {
    qwenwork_accounts::apply(edition, &account_id).await
}

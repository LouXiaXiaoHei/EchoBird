use crate::services::dumate_accounts::{self, Account, LoginStart};

#[tauri::command]
pub async fn list_dumate_accounts() -> Result<Vec<Account>, String> {
    dumate_accounts::list().await
}
#[tauri::command]
pub async fn start_dumate_login(app: tauri::AppHandle) -> Result<LoginStart, String> {
    dumate_accounts::start_login(app).await
}
#[tauri::command]
pub async fn poll_dumate_login(
    app: tauri::AppHandle,
    login_id: String,
) -> Result<Option<Account>, String> {
    dumate_accounts::poll_login(&app, &login_id).await
}
#[tauri::command]
pub async fn cancel_dumate_login(app: tauri::AppHandle, login_id: String) -> Result<(), String> {
    dumate_accounts::cancel_login(&app, &login_id)
}
#[tauri::command]
pub async fn refresh_dumate_account_quota(account_id: String) -> Result<Account, String> {
    dumate_accounts::refresh(&account_id).await
}
#[tauri::command]
pub async fn claim_dumate_daily_credits(account_id: String) -> Result<Account, String> {
    dumate_accounts::claim(&account_id).await
}
#[tauri::command]
pub async fn delete_dumate_account(account_id: String) -> Result<(), String> {
    crate::services::account_model_sources::finish_account_deletion(
        crate::models::model::AccountProvider::Dumate,
        &account_id,
        dumate_accounts::delete(&account_id).await,
    )
}
#[tauri::command]
pub async fn switch_dumate_account(account_id: String) -> Result<Account, String> {
    dumate_accounts::apply(&account_id).await
}

use crate::models::model::ModelConfig;
use crate::services::smart_router::{self, PublicActivity, PublicConfig};

#[tauri::command]
pub async fn set_smart_router_enabled(enabled: bool) -> Result<PublicConfig, String> {
    smart_router::profiles::set_master_enabled(enabled).await
}

#[tauri::command]
pub fn set_smart_router_candidate_enabled(
    internal_id: String,
    enabled: bool,
    router_id: Option<String>,
) -> Result<PublicConfig, String> {
    smart_router::profiles::checked(router_id, || {
        smart_router::set_candidate_enabled(&internal_id, enabled)
    })?
}

#[tauri::command]
pub fn get_smart_router_config(router_id: Option<String>) -> Result<PublicConfig, String> {
    smart_router::profiles::checked(router_id, smart_router::get_public_config)?
}

#[tauri::command]
pub fn get_smart_router_activity(router_id: Option<String>) -> Result<PublicActivity, String> {
    smart_router::profiles::checked(router_id, smart_router::get_public_activity)
}

#[tauri::command]
pub fn set_smart_router_candidates(
    candidate_ids: Vec<String>,
    router_id: Option<String>,
) -> Result<PublicConfig, String> {
    smart_router::profiles::checked(router_id, || smart_router::set_candidate_ids(candidate_ids))?
}

#[tauri::command]
pub fn get_smart_router_candidates(router_id: Option<String>) -> Result<Vec<ModelConfig>, String> {
    smart_router::profiles::checked(router_id, smart_router::get_candidate_models)
}

#[tauri::command]
pub fn remove_smart_router_candidate(
    internal_id: String,
    router_id: Option<String>,
) -> Result<PublicConfig, String> {
    smart_router::profiles::checked(router_id, || smart_router::remove_candidate(&internal_id))?
}

#[tauri::command]
pub fn list_smart_routers() -> Result<Vec<PublicConfig>, String> {
    smart_router::profiles::list()
}
#[tauri::command]
pub fn get_smart_router_settings(
    router_id: String,
) -> Result<smart_router::profiles::Settings, String> {
    smart_router::profiles::settings(router_id)
}
#[tauri::command]
pub fn get_smart_router_available_port(exclude_port: Option<u16>) -> Result<u16, String> {
    smart_router::profiles::available_port(exclude_port)
}
#[tauri::command]
pub async fn save_smart_router(
    settings: smart_router::profiles::Settings,
) -> Result<PublicConfig, String> {
    smart_router::profiles::save(settings).await
}
#[tauri::command]
pub async fn delete_smart_router(router_id: String) -> Result<(), String> {
    smart_router::profiles::delete(router_id).await
}
#[tauri::command]
pub async fn set_smart_router_group_enabled(
    router_id: String,
    enabled: bool,
) -> Result<PublicConfig, String> {
    smart_router::profiles::set_group_enabled(&router_id, enabled).await
}

#[tauri::command]
pub(crate) async fn test_smart_router_candidate(
    internal_id: String,
    model_id: String,
    request_id: String,
    batch_id: Option<String>,
    router_id: Option<String>,
) -> smart_router::ProbeResult {
    smart_router::probe_candidate(
        &internal_id,
        &model_id,
        &request_id,
        batch_id.as_deref(),
        router_id,
    )
    .await
}

#[tauri::command]
pub fn cancel_smart_router_probe(request_id: String) {
    smart_router::cancel_probe(&request_id);
}

// Agent Commands — Tauri IPC commands for the MotherAgent frontend

use crate::commands::ssh_commands::SSHPool;
use crate::services::agent_loop::{self, AgentEvent, AgentRequest, SharedSessionMap};
use crate::services::agent_memory::{MemoryRecord, MemoryScope, MemoryStore};
use tauri::{AppHandle, State};

/// Send a message to the agent. The agent will process asynchronously
/// and emit `agent_event` events to the frontend.
#[tauri::command]
pub async fn agent_send_message(
    app: AppHandle,
    session_map: State<'_, SharedSessionMap>,
    ssh_pool: State<'_, SSHPool>,
    request: AgentRequest,
) -> Result<String, String> {
    let server_key = request.session_key();

    let map_clone = session_map.inner().clone();
    let pool_clone = ssh_pool.inner().clone();

    // Reserve the session before spawning so abort/reset also see pending startup.
    let session_id = {
        let mut map = session_map.lock().await;
        if !map.contains_key(&server_key) {
            map.insert(
                server_key.clone(),
                agent_loop::AgentSession::restore(&server_key)?,
            );
        }
        let sess = map.get_mut(&server_key).unwrap();
        sess.prepare_run()?;
        sess.id.clone()
    };

    // Spawn agent as background task
    let app_clone = app.clone();
    let conversation_id = request.conversation_id.clone();
    let request_id = request.request_id.clone();
    tokio::spawn(async move {
        if let Err(e) = agent_loop::run_agent(app, request, map_clone.clone(), pool_clone).await {
            log::error!("[AgentCommand] Agent error: {}", e);
            if let Some(sess) = map_clone.lock().await.get_mut(&server_key) {
                sess.running = false;
            }
            // Safety net: ensure frontend is never left stuck in isProcessing
            agent_loop::emit_event(
                &app_clone,
                conversation_id.as_deref(),
                request_id.as_deref(),
                AgentEvent::Error { message: e },
            );
            agent_loop::emit_event(
                &app_clone,
                conversation_id.as_deref(),
                request_id.as_deref(),
                AgentEvent::Done {},
            );
        }
    });

    Ok(session_id)
}

/// Abort the current agent execution
#[tauri::command]
pub async fn agent_abort(
    session_map: State<'_, SharedSessionMap>,
    server_key: String,
) -> Result<bool, String> {
    let mut map = session_map.lock().await;
    if let Some(sess) = map.get_mut(&server_key) {
        if sess.running {
            sess.cancel();
            log::info!("[AgentCommand] Agent aborted for server: {}", server_key);
            return Ok(true);
        }
    }
    Ok(false)
}

/// Reset the agent session for a specific server (clear conversation history)
#[tauri::command]
pub async fn agent_reset(
    session_map: State<'_, SharedSessionMap>,
    server_key: String,
) -> Result<String, String> {
    let mut map = session_map.lock().await;
    if map.get(&server_key).is_some_and(|sess| sess.running) {
        return Err("Agent is still processing a request".into());
    }
    // Keep the live history intact if removing its persisted copy fails.
    agent_loop::clear_session_from_disk(&server_key)?;
    let sess = map
        .entry(server_key.clone())
        .or_insert_with(agent_loop::AgentSession::new);
    sess.reset()?;
    log::info!(
        "[AgentCommand] Session reset for server {}: {}",
        server_key,
        sess.id
    );
    Ok(sess.id.clone())
}

#[tauri::command]
pub fn agent_memory_list() -> Result<Vec<MemoryRecord>, String> {
    MemoryStore::standard().list()
}

#[tauri::command]
pub fn agent_memory_save(
    scope: MemoryScope,
    target: Option<String>,
    text: String,
    source_conversation_id: Option<String>,
) -> Result<MemoryRecord, String> {
    MemoryStore::standard().remember(
        scope,
        target.as_deref(),
        &text,
        source_conversation_id.as_deref(),
    )
}

#[tauri::command]
pub fn agent_memory_update(id: String, text: String) -> Result<MemoryRecord, String> {
    MemoryStore::standard().update(&id, &text)
}

#[tauri::command]
pub fn agent_memory_delete(id: String) -> Result<(), String> {
    MemoryStore::standard().forget(&id)
}

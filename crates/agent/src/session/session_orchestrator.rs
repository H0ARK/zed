use anyhow::Result;
use chrono::{DateTime, Utc};
use gpui::{App, Context, Entity, EventEmitter, Task};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::ThreadStore;
use crate::session::{AgentSessionManager, SessionConfig, SessionId, SessionStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    pub max_concurrent_sessions: usize,
    pub coordination_timeout_seconds: u64,
    pub enable_session_sharing: bool,
    pub orchestrator_model: String,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_sessions: 5,
            coordination_timeout_seconds: 30,
            enable_session_sharing: true,
            orchestrator_model: "gpt-4".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum OrchestratorEvent {
    SessionCreated(SessionId),
    SessionRemoved(SessionId),
    SessionActivated(SessionId),
    MainAIResponse(String),
    CoordinationStarted,
    CoordinationCompleted,
    CoordinationFailed(String),
    SessionStatusChanged(SessionId, SessionStatus),
}

pub struct SessionOrchestrator {
    session_manager: Entity<AgentSessionManager>,
    #[allow(dead_code)]
    thread_store: Entity<ThreadStore>,
    pub config: OrchestratorConfig,
    active_coordination_task: Option<Task<Result<()>>>,
    coordination_history: Vec<CoordinationEntry>,
    session_communication_channels: HashMap<SessionId, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct CoordinationEntry {
    pub timestamp: DateTime<Utc>,
    pub session_id: Option<SessionId>,
    pub message: String,
    pub entry_type: CoordinationType,
}

#[derive(Debug, Clone)]
pub enum CoordinationType {
    SessionCreated,
    SessionRemoved,
    SessionActivated,
    CoordinationRequest,
    CoordinationResponse,
    AgentCommunication,
    Error,
}

impl EventEmitter<OrchestratorEvent> for SessionOrchestrator {}

impl SessionOrchestrator {
    pub fn new(
        session_manager: Entity<AgentSessionManager>,
        thread_store: Entity<ThreadStore>,
        config: OrchestratorConfig,
        cx: &mut Context<Self>,
    ) -> Self {
        let orchestrator = Self {
            session_manager: session_manager.clone(),
            thread_store,
            config,
            active_coordination_task: None,
            coordination_history: Vec::new(),
            session_communication_channels: HashMap::new(),
        };

        // Subscribe to session manager events
        cx.subscribe(&session_manager, Self::handle_session_manager_event)
            .detach();

        orchestrator
    }

    pub fn session_count(&self, cx: &App) -> usize {
        self.session_manager.read(cx).list_sessions().len()
    }

    pub fn create_session(
        &mut self,
        name: String,
        config: SessionConfig,
        cx: &mut Context<Self>,
    ) -> Result<SessionId> {
        let session_id = self
            .session_manager
            .update(cx, |manager, cx| manager.create_session(&name, config, cx))?;

        self.add_coordination_entry(
            Some(session_id.clone()),
            format!("Created session: {}", name),
            CoordinationType::SessionCreated,
        );

        cx.emit(OrchestratorEvent::SessionCreated(session_id.clone()));
        Ok(session_id)
    }

    pub fn remove_session(&mut self, session_id: &SessionId, cx: &mut Context<Self>) -> Result<()> {
        self.session_manager
            .update(cx, |manager, cx| manager.remove_session(session_id, cx))?;

        self.session_communication_channels.remove(session_id);
        self.add_coordination_entry(
            Some(session_id.clone()),
            format!("Removed session: {}", session_id),
            CoordinationType::SessionRemoved,
        );

        cx.emit(OrchestratorEvent::SessionRemoved(session_id.clone()));
        Ok(())
    }

    pub fn activate_session(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.session_manager.update(cx, |manager, cx| {
            manager.activate_session(session_id.clone(), cx)
        })?;

        self.add_coordination_entry(
            Some(session_id.clone()),
            format!("Activated session: {}", session_id),
            CoordinationType::SessionActivated,
        );

        cx.emit(OrchestratorEvent::SessionActivated(session_id));
        Ok(())
    }

    pub fn coordinate_sessions(&mut self, task: String, cx: &mut Context<Self>) -> Result<()> {
        if self.active_coordination_task.is_some() {
            return Err(anyhow::anyhow!("Coordination already in progress"));
        }

        cx.emit(OrchestratorEvent::CoordinationStarted);
        self.add_coordination_entry(
            None,
            format!("Starting coordination: {}", task),
            CoordinationType::CoordinationRequest,
        );

        let session_manager = self.session_manager.clone();
        let orchestrator_handle = cx.entity().downgrade();

        let coordination_task = cx.spawn(async move |_handle, mut cx| {
            // Simulate coordination logic
            let result = Self::perform_coordination(session_manager, task, &mut cx).await;

            if let Some(orchestrator) = orchestrator_handle.upgrade() {
                orchestrator.update(cx, |orchestrator, cx| {
                    orchestrator.active_coordination_task = None;
                    match result {
                        Ok(response) => {
                            orchestrator.add_coordination_entry(
                                None,
                                format!("Coordination completed: {}", response),
                                CoordinationType::CoordinationResponse,
                            );
                            cx.emit(OrchestratorEvent::CoordinationCompleted);
                            cx.emit(OrchestratorEvent::MainAIResponse(response));
                        }
                        Err(err) => {
                            orchestrator.add_coordination_entry(
                                None,
                                format!("Coordination failed: {}", err),
                                CoordinationType::Error,
                            );
                            cx.emit(OrchestratorEvent::CoordinationFailed(err.to_string()));
                        }
                    }
                })?;
            }

            anyhow::Ok(())
        });

        self.active_coordination_task = Some(coordination_task);
        Ok(())
    }

    async fn perform_coordination(
        session_manager: Entity<AgentSessionManager>,
        task: String,
        cx: &mut gpui::AsyncApp,
    ) -> Result<String> {
        // This is a simplified coordination logic
        // In a real implementation, this would involve:
        // 1. Analyzing the task
        // 2. Determining which sessions can contribute
        // 3. Orchestrating communication between sessions
        // 4. Aggregating results

        let session_ids = session_manager.read_with(cx, |manager, _| manager.list_sessions())?;

        if session_ids.is_empty() {
            return Ok("No active sessions to coordinate".to_string());
        }

        // Simulate coordination work
        gpui::Timer::after(std::time::Duration::from_secs(2)).await;

        Ok(format!(
            "Coordinated {} sessions for task: {}. Analysis complete.",
            session_ids.len(),
            task
        ))
    }

    pub fn send_message_between_sessions(
        &mut self,
        from_session: &SessionId,
        to_session: &SessionId,
        message: String,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        // Validate sessions exist
        let sessions = self.session_manager.read(cx).list_sessions();
        if !sessions.contains(from_session) || !sessions.contains(to_session) {
            return Err(anyhow::anyhow!("One or both sessions do not exist"));
        }

        // Add to communication channel
        self.session_communication_channels
            .entry(to_session.clone())
            .or_insert_with(Vec::new)
            .push(format!("From {}: {}", from_session, message));

        self.add_coordination_entry(
            Some(from_session.clone()),
            format!("Sent message to {}: {}", to_session, message),
            CoordinationType::AgentCommunication,
        );

        Ok(())
    }

    pub fn get_session_messages(&self, session_id: &SessionId) -> Vec<String> {
        self.session_communication_channels
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn coordination_history(&self) -> &[CoordinationEntry] {
        &self.coordination_history
    }

    pub fn is_coordinating(&self) -> bool {
        self.active_coordination_task.is_some()
    }

    fn add_coordination_entry(
        &mut self,
        session_id: Option<SessionId>,
        message: String,
        entry_type: CoordinationType,
    ) {
        let entry = CoordinationEntry {
            timestamp: Utc::now(),
            session_id,
            message,
            entry_type,
        };

        self.coordination_history.push(entry);

        // Keep only the last 100 entries
        if self.coordination_history.len() > 100 {
            self.coordination_history.remove(0);
        }
    }

    fn handle_session_manager_event(
        &mut self,
        _session_manager: Entity<AgentSessionManager>,
        event: &crate::session::SessionManagerEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            crate::session::SessionManagerEvent::SessionCreated(session_id) => {
                self.add_coordination_entry(
                    Some(session_id.clone()),
                    "Session created via manager".to_string(),
                    CoordinationType::SessionCreated,
                );
                cx.emit(OrchestratorEvent::SessionCreated(session_id.clone()));
            }
            crate::session::SessionManagerEvent::SessionRemoved(session_id) => {
                self.session_communication_channels.remove(session_id);
                self.add_coordination_entry(
                    Some(session_id.clone()),
                    "Session removed via manager".to_string(),
                    CoordinationType::SessionRemoved,
                );
                cx.emit(OrchestratorEvent::SessionRemoved(session_id.clone()));
            }
            crate::session::SessionManagerEvent::SessionActivated(session_id) => {
                self.add_coordination_entry(
                    Some(session_id.clone()),
                    "Session activated via manager".to_string(),
                    CoordinationType::SessionActivated,
                );
                cx.emit(OrchestratorEvent::SessionActivated(session_id.clone()));
            }
            // Note: SessionStatusChanged event doesn't exist in SessionManagerEvent
            _ => {}
        }
    }
}

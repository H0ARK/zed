use std::collections::HashMap;

use anyhow::{Result, anyhow};
use gpui::{App, AppContext, Context, Entity, EventEmitter, Global};
use project::Project;
use serde::{Deserialize, Serialize};

use crate::ThreadStore;
use crate::session::agent_session::{AgentSession, SessionConfig, SessionId, SessionStatus};

#[derive(Clone)]
pub struct GlobalSessionManager(Entity<AgentSessionManager>);

impl Global for GlobalSessionManager {}

pub struct AgentSessionManager {
    sessions: HashMap<SessionId, Entity<AgentSession>>,
    active_session_id: Option<SessionId>,
    thread_store: Entity<ThreadStore>,
    #[allow(dead_code)]
    project: Entity<Project>,
}

impl EventEmitter<SessionManagerEvent> for AgentSessionManager {}

#[derive(Debug, Clone)]
pub enum SessionManagerEvent {
    SessionCreated(SessionId),
    SessionRemoved(SessionId),
    SessionActivated(SessionId),
    SessionDeactivated(SessionId),
    SessionUpdated(SessionId),
}

impl AgentSessionManager {
    pub fn new(
        thread_store: Entity<ThreadStore>,
        project: Entity<Project>,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            sessions: HashMap::new(),
            active_session_id: None,
            thread_store,
            project,
        }
    }

    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalSessionManager>()
            .map(|manager| manager.0.clone())
    }

    pub fn set_global(manager: Entity<Self>, cx: &mut App) {
        cx.set_global(GlobalSessionManager(manager));
    }

    pub fn create_session(
        &mut self,
        name: impl Into<gpui::SharedString>,
        config: SessionConfig,
        cx: &mut Context<Self>,
    ) -> Result<SessionId> {
        let name = name.into();
        let session_id = if let Some(provider) = &config.provider {
            SessionId::from_name(&format!("{}_{}", provider, name))
        } else {
            SessionId::from_name(&name)
        };

        // Create a new thread for this session
        let thread = self.thread_store.update(cx, |store, cx| {
            store.create_thread(cx)
        });

        // Create the agent session
        let agent_session = cx.new(|_cx| {
            AgentSession::with_id(session_id.clone(), name.clone(), thread, config)
        });

        // Add to session manager
        self.sessions.insert(session_id.clone(), agent_session);
        cx.emit(SessionManagerEvent::SessionCreated(session_id.clone()));

        Ok(session_id)
    }

    pub fn get_session(&self, session_id: &SessionId) -> Option<&Entity<AgentSession>> {
        self.sessions.get(session_id)
    }

    pub fn get_session_by_name(&self, name: &str, cx: &App) -> Option<&Entity<AgentSession>> {
        self.sessions.values().find(|session| {
            session.read(cx).name() == name
        })
    }

    pub fn list_sessions(&self) -> Vec<SessionId> {
        self.sessions.keys().cloned().collect()
    }

    pub fn active_session(&self) -> Option<&Entity<AgentSession>> {
        self.active_session_id
            .as_ref()
            .and_then(|id| self.sessions.get(id))
    }

    pub fn active_session_id(&self) -> Option<&SessionId> {
        self.active_session_id.as_ref()
    }

    pub fn activate_session(&mut self, session_id: SessionId, cx: &mut Context<Self>) -> Result<()> {
        if !self.sessions.contains_key(&session_id) {
            return Err(anyhow!("Session not found: {}", session_id));
        }

        if let Some(old_id) = &self.active_session_id {
            if old_id != &session_id {
                cx.emit(SessionManagerEvent::SessionDeactivated(old_id.clone()));
            }
        }

        self.active_session_id = Some(session_id.clone());
        cx.emit(SessionManagerEvent::SessionActivated(session_id));
        Ok(())
    }

    pub fn deactivate_session(&mut self, cx: &mut Context<Self>) {
        if let Some(session_id) = self.active_session_id.take() {
            cx.emit(SessionManagerEvent::SessionDeactivated(session_id));
        }
    }

    pub fn remove_session(&mut self, session_id: &SessionId, cx: &mut Context<Self>) -> Result<()> {
        if let Some(_session) = self.sessions.remove(session_id) {
            if self.active_session_id.as_ref() == Some(session_id) {
                self.active_session_id = None;
            }
            cx.emit(SessionManagerEvent::SessionRemoved(session_id.clone()));
            Ok(())
        } else {
            Err(anyhow!("Session not found: {}", session_id))
        }
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn has_active_sessions(&self, cx: &App) -> bool {
        self.sessions.values().any(|session| {
            session.read(cx).is_active()
        })
    }

    pub fn get_sessions_by_status(&self, status: &SessionStatus, cx: &App) -> Vec<&Entity<AgentSession>> {
        self.sessions
            .values()
            .filter(|session| {
                std::mem::discriminant(session.read(cx).status()) == 
                std::mem::discriminant(status)
            })
            .collect()
    }

    pub fn send_message_to_session(
        &mut self,
        session_id: &SessionId,
        _message: String,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let session = self.sessions.get(session_id)
            .ok_or_else(|| anyhow!("Session not found: {}", session_id))?;

        session.update(cx, |session, cx| {
            session.update_status(SessionStatus::Thinking, cx);
            // TODO: Integrate with actual thread message sending logic
        });

        Ok(())
    }

    pub fn clone_session(
        &mut self,
        source_session_id: &SessionId,
        new_name: impl Into<gpui::SharedString>,
        cx: &mut Context<Self>,
    ) -> Result<SessionId> {
        let source_session = self.sessions.get(source_session_id)
            .ok_or_else(|| anyhow!("Source session not found: {}", source_session_id))?;

        let new_name = new_name.into();
        let config = source_session.read(cx).config().clone();

        self.create_session(new_name, config, cx)
    }

    pub fn get_session_stats(&self, cx: &App) -> SessionManagerStats {
        let sessions: Vec<_> = self.sessions.values().collect();
        let total = sessions.len();
        let active = sessions.iter().filter(|s| s.read(cx).is_active()).count();
        let idle = sessions.iter().filter(|s| matches!(s.read(cx).status(), SessionStatus::Idle)).count();
        let error = sessions.iter().filter(|s| matches!(s.read(cx).status(), SessionStatus::Error(_))).count();

        SessionManagerStats {
            total_sessions: total,
            active_sessions: active,
            idle_sessions: idle,
            error_sessions: error,
            has_active_session: self.active_session_id.is_some(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionManagerStats {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub idle_sessions: usize,
    pub error_sessions: usize,
    pub has_active_session: bool,
}
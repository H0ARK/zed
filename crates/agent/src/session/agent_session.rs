use std::sync::Arc;

use chrono::{DateTime, Utc};
use gpui::{Entity, EventEmitter, SharedString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::thread::Thread;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(Arc<str>);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string().into())
    }
    
    pub fn from_name(name: &str) -> Self {
        Self(name.into())
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for SessionId {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionStatus {
    Idle,
    Thinking,
    Responding,
    WaitingForUser,
    Error(SharedString),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub working_directory: Option<String>,
    pub auto_continue: bool,
    pub context_sharing: bool,
}

#[allow(clippy::derivable_impls)]
impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            provider: None,
            model: None,
            max_tokens: None,
            temperature: None,
            working_directory: None,
            auto_continue: false,
            context_sharing: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: SessionId,
    pub name: SharedString,
    pub created_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub config: SessionConfig,
    pub message_count: usize,
    pub tags: Vec<SharedString>,
}

pub struct AgentSession {
    pub metadata: SessionMetadata,
    pub thread: Entity<Thread>,
    pub status: SessionStatus,
}

impl EventEmitter<SessionEvent> for AgentSession {}

#[derive(Debug, Clone)]
pub enum SessionEvent {
    StatusChanged(SessionStatus),
    MessageAdded,
    ConfigUpdated(SessionConfig),
    Renamed(SharedString),
}

impl AgentSession {
    pub fn new(
        name: impl Into<SharedString>,
        thread: Entity<Thread>,
        config: SessionConfig,
    ) -> Self {
        let now = Utc::now();
        Self {
            metadata: SessionMetadata {
                id: SessionId::new(),
                name: name.into(),
                created_at: now,
                last_active: now,
                config,
                message_count: 0,
                tags: Vec::new(),
            },
            thread,
            status: SessionStatus::Idle,
        }
    }

    pub fn with_id(
        id: SessionId,
        name: impl Into<SharedString>,
        thread: Entity<Thread>,
        config: SessionConfig,
    ) -> Self {
        let now = Utc::now();
        Self {
            metadata: SessionMetadata {
                id,
                name: name.into(),
                created_at: now,
                last_active: now,
                config,
                message_count: 0,
                tags: Vec::new(),
            },
            thread,
            status: SessionStatus::Idle,
        }
    }

    pub fn id(&self) -> &SessionId {
        &self.metadata.id
    }

    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    pub fn status(&self) -> &SessionStatus {
        &self.status
    }

    pub fn config(&self) -> &SessionConfig {
        &self.metadata.config
    }

    pub fn thread(&self) -> &Entity<Thread> {
        &self.thread
    }

    pub fn update_status(&mut self, status: SessionStatus, cx: &mut gpui::Context<Self>) {
        self.status = status.clone();
        self.metadata.last_active = Utc::now();
        cx.emit(SessionEvent::StatusChanged(status));
    }

    pub fn update_config(&mut self, config: SessionConfig, cx: &mut gpui::Context<Self>) {
        self.metadata.config = config.clone();
        self.metadata.last_active = Utc::now();
        cx.emit(SessionEvent::ConfigUpdated(config));
    }

    pub fn rename(&mut self, name: impl Into<SharedString>, cx: &mut gpui::Context<Self>) {
        let name = name.into();
        self.metadata.name = name.clone();
        self.metadata.last_active = Utc::now();
        cx.emit(SessionEvent::Renamed(name));
    }

    pub fn add_tag(&mut self, tag: impl Into<SharedString>) {
        let tag = tag.into();
        if !self.metadata.tags.contains(&tag) {
            self.metadata.tags.push(tag);
            self.metadata.last_active = Utc::now();
        }
    }

    pub fn remove_tag(&mut self, tag: &str) {
        self.metadata.tags.retain(|t| t.as_ref() != tag);
        self.metadata.last_active = Utc::now();
    }

    pub fn increment_message_count(&mut self, cx: &mut gpui::Context<Self>) {
        self.metadata.message_count += 1;
        self.metadata.last_active = Utc::now();
        cx.emit(SessionEvent::MessageAdded);
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, SessionStatus::Thinking | SessionStatus::Responding)
    }

    pub fn age_minutes(&self) -> i64 {
        let now = Utc::now();
        (now - self.metadata.created_at).num_minutes()
    }

    pub fn last_active_minutes(&self) -> i64 {
        let now = Utc::now();
        (now - self.metadata.last_active).num_minutes()
    }
}
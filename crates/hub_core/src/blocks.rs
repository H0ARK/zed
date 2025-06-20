//! Block definitions and management

use crate::types::*;
use serde::{Deserialize, Serialize};

/// A block represents an interactive UI component in The Hub
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub id: BlockId,
    pub session_id: SessionId,
    pub title: String,
    pub content: BlockContent,
    pub state: BlockState,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Block state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockState {
    Loading,
    Ready,
    Error { message: String },
    Interactive,
}

/// Block update message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockUpdate {
    pub block_id: BlockId,
    pub content: Option<BlockContent>,
    pub state: Option<BlockState>,
}

impl Block {
    pub fn new(session_id: SessionId, title: String) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: BlockId(uuid::Uuid::new_v4().to_string()),
            session_id,
            title,
            content: BlockContent::Text("Loading...".to_string()),
            state: BlockState::Loading,
            created_at: now,
            updated_at: now,
        }
    }
    
    pub fn update(&mut self, update: BlockUpdate) {
        if let Some(content) = update.content {
            self.content = content;
        }
        if let Some(state) = update.state {
            self.state = state;
        }
        self.updated_at = chrono::Utc::now();
    }
}
//! Block management system for The Hub
//!
//! This module manages the lifecycle of blocks, including creation,
//! updates, persistence, and cleanup.

use crate::block::{Block, BlockId, BlockStatus};
use anyhow::Result;
use std::collections::HashMap;
use tokio::sync::RwLock;
use std::sync::Arc;

/// Manager for all active blocks
pub struct BlockManager {
    blocks: Arc<RwLock<HashMap<BlockId, Block>>>,
    block_history: Arc<RwLock<Vec<Block>>>,
}

impl BlockManager {
    /// Create a new block manager
    pub fn new() -> Self {
        Self {
            blocks: Arc::new(RwLock::new(HashMap::new())),
            block_history: Arc::new(RwLock::new(Vec::new())),
        }
    }
    
    /// Create a new block
    pub async fn create_block(
        &self,
        command: String,
        args: Vec<String>,
        working_directory: String,
        session_id: String,
    ) -> Result<BlockId> {
        let block_id = uuid::Uuid::new_v4().to_string();
        let block = Block::new(
            block_id.clone(),
            command,
            args,
            working_directory,
            session_id,
        );
        
        let mut blocks = self.blocks.write().await;
        blocks.insert(block_id.clone(), block);
        
        Ok(block_id)
    }
    
    /// Get a block by ID
    pub async fn get_block(&self, block_id: &BlockId) -> Option<Block> {
        let blocks = self.blocks.read().await;
        blocks.get(block_id).cloned()
    }
    
    /// Update a block
    pub async fn update_block(&self, block: Block) -> Result<()> {
        let mut blocks = self.blocks.write().await;
        blocks.insert(block.id.clone(), block);
        Ok(())
    }
    
    /// Remove a block
    pub async fn remove_block(&self, block_id: &BlockId) -> Result<Option<Block>> {
        let mut blocks = self.blocks.write().await;
        
        if let Some(block) = blocks.remove(block_id) {
            // Archive completed blocks
            let mut history = self.block_history.write().await;
            history.push(block.clone());
            
            Ok(Some(block))
        } else {
            Ok(None)
        }
    }
    
    /// Get all active blocks
    pub async fn get_active_blocks(&self) -> Vec<Block> {
        let blocks = self.blocks.read().await;
        blocks.values()
            .filter(|block| block.is_active())
            .cloned()
            .collect()
    }
    
    /// Get all blocks for a session
    pub async fn get_session_blocks(&self, session_id: &str) -> Vec<Block> {
        let blocks = self.blocks.read().await;
        blocks.values()
            .filter(|block| block.metadata.session_id == session_id)
            .cloned()
            .collect()
    }
    
    /// Clean up completed blocks older than the specified duration
    pub async fn cleanup_old_blocks(&self, max_age_hours: u64) -> Result<usize> {
        let cutoff = chrono::Utc::now() - chrono::Duration::hours(max_age_hours as i64);
        let mut blocks = self.blocks.write().await;
        let mut history = self.block_history.write().await;
        
        let mut to_remove = Vec::new();
        
        for (id, block) in blocks.iter() {
            if !block.is_active() && block.metadata.updated_at < cutoff {
                to_remove.push(id.clone());
                history.push(block.clone());
            }
        }
        
        let removed_count = to_remove.len();
        for id in to_remove {
            blocks.remove(&id);
        }
        
        Ok(removed_count)
    }
    
    /// Get block statistics
    pub async fn get_statistics(&self) -> BlockStatistics {
        let blocks = self.blocks.read().await;
        let history = self.block_history.read().await;
        
        let mut stats = BlockStatistics {
            total_active: blocks.len(),
            running: 0,
            completed: 0,
            failed: 0,
            paused: 0,
            total_historical: history.len(),
        };
        
        for block in blocks.values() {
            match block.status {
                BlockStatus::Running => stats.running += 1,
                BlockStatus::Completed { .. } => stats.completed += 1,
                BlockStatus::Failed { .. } => stats.failed += 1,
                BlockStatus::Paused => stats.paused += 1,
                BlockStatus::Cancelled => stats.failed += 1,
            }
        }
        
        stats
    }
}

impl Default for BlockManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about block usage
#[derive(Debug, Clone)]
pub struct BlockStatistics {
    pub total_active: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub paused: usize,
    pub total_historical: usize,
}
//! Settings and configuration for The Hub

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Hub platform settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubSettings {
    pub protocol: ProtocolSettings,
    pub ui: UiSettings,
    pub ai: AiSettings,
    pub developer: DeveloperSettings,
}

/// Protocol configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolSettings {
    pub port: u16,
    pub timeout_seconds: u64,
    pub max_message_size: usize,
}

/// UI configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    pub theme: String,
    pub font_size: f32,
    pub max_blocks_per_session: usize,
    pub auto_scroll: bool,
}

/// AI integration settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettings {
    pub enabled: bool,
    pub provider: String,
    pub suggestions_enabled: bool,
    pub auto_help: bool,
}

/// Developer tools settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeveloperSettings {
    pub debug_mode: bool,
    pub protocol_logging: bool,
    pub sdk_path: Option<PathBuf>,
}

impl Default for HubSettings {
    fn default() -> Self {
        Self {
            protocol: ProtocolSettings {
                port: 8765,
                timeout_seconds: 30,
                max_message_size: 1024 * 1024, // 1MB
            },
            ui: UiSettings {
                theme: "dark".to_string(),
                font_size: 14.0,
                max_blocks_per_session: 100,
                auto_scroll: true,
            },
            ai: AiSettings {
                enabled: true,
                provider: "anthropic".to_string(),
                suggestions_enabled: true,
                auto_help: true,
            },
            developer: DeveloperSettings {
                debug_mode: false,
                protocol_logging: false,
                sdk_path: None,
            },
        }
    }
}
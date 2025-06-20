//! AI-powered command suggestions and optimizations
//!
//! This module provides intelligent suggestions for improving command usage
//! and discovering new features.

use anyhow::Result;
use hub_protocol::messages::*;
use serde::{Deserialize, Serialize};
use crate::context_analysis::{CommandContext, ContextAnalysis};

/// A suggestion for improving command usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSuggestion {
    pub suggestion_type: SuggestionType,
    pub title: String,
    pub description: String,
    pub command: Option<String>,
    pub confidence: f64,
    pub priority: SuggestionPriority,
}

/// Types of suggestions that can be made
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuggestionType {
    Optimization,
    Alternative,
    Feature,
    Safety,
    Efficiency,
    Learning,
}

/// Priority level of suggestions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuggestionPriority {
    Low,
    Medium,
    High,
    Critical,
}

/// Suggestion generator
pub struct SuggestionEngine {
    // This would integrate with AI models
}

impl SuggestionEngine {
    pub fn new() -> Self {
        Self {}
    }
    
    /// Generate suggestions based on command context and analysis
    pub async fn generate_suggestions(
        &self,
        context: &CommandContext,
        analysis: &ContextAnalysis,
    ) -> Result<Vec<CommandSuggestion>> {
        let mut suggestions = Vec::new();
        
        // Example suggestions based on common patterns
        suggestions.extend(self.suggest_alternatives(context)?);
        suggestions.extend(self.suggest_optimizations(context)?);
        suggestions.extend(self.suggest_safety_improvements(context)?);
        
        Ok(suggestions)
    }
    
    fn suggest_alternatives(&self, context: &CommandContext) -> Result<Vec<CommandSuggestion>> {
        let mut suggestions = Vec::new();
        
        // Example: Suggest modern alternatives
        if context.command == "ls" {
            suggestions.push(CommandSuggestion {
                suggestion_type: SuggestionType::Alternative,
                title: "Modern alternative".to_string(),
                description: "Consider using 'exa' or 'lsd' for enhanced output".to_string(),
                command: Some("exa -la".to_string()),
                confidence: 0.7,
                priority: SuggestionPriority::Low,
            });
        }
        
        if context.command == "cat" {
            suggestions.push(CommandSuggestion {
                suggestion_type: SuggestionType::Alternative,
                title: "Better file viewing".to_string(),
                description: "Try 'bat' for syntax highlighting and line numbers".to_string(),
                command: Some("bat".to_string()),
                confidence: 0.8,
                priority: SuggestionPriority::Medium,
            });
        }
        
        Ok(suggestions)
    }
    
    fn suggest_optimizations(&self, context: &CommandContext) -> Result<Vec<CommandSuggestion>> {
        let mut suggestions = Vec::new();
        
        // Example: Suggest flag optimizations
        if context.command.starts_with("git") && context.args.contains(&"commit".to_string()) {
            if !context.args.contains(&"-m".to_string()) {
                suggestions.push(CommandSuggestion {
                    suggestion_type: SuggestionType::Optimization,
                    title: "Add commit message".to_string(),
                    description: "Use -m flag to specify commit message inline".to_string(),
                    command: Some("git commit -m \"your message\"".to_string()),
                    confidence: 0.9,
                    priority: SuggestionPriority::High,
                });
            }
        }
        
        Ok(suggestions)
    }
    
    fn suggest_safety_improvements(&self, context: &CommandContext) -> Result<Vec<CommandSuggestion>> {
        let mut suggestions = Vec::new();
        
        // Example: Safety suggestions
        if context.command.contains("rm") && !context.args.contains(&"-i".to_string()) {
            suggestions.push(CommandSuggestion {
                suggestion_type: SuggestionType::Safety,
                title: "Interactive deletion".to_string(),
                description: "Consider using -i flag for interactive confirmation".to_string(),
                command: Some("rm -i".to_string()),
                confidence: 0.85,
                priority: SuggestionPriority::High,
            });
        }
        
        Ok(suggestions)
    }
    
    /// Generate real-time suggestions as user types
    pub async fn suggest_completion(
        &self,
        partial_command: &str,
        context: &CommandContext,
    ) -> Result<Vec<String>> {
        // Basic completion suggestions
        let mut completions = Vec::new();
        
        if partial_command.starts_with("git ") {
            let git_commands = vec![
                "add", "commit", "push", "pull", "clone", "status", "log", "diff",
                "branch", "checkout", "merge", "rebase", "reset", "stash",
            ];
            
            for cmd in git_commands {
                if cmd.starts_with(&partial_command[4..]) {
                    completions.push(format!("git {}", cmd));
                }
            }
        }
        
        Ok(completions)
    }
}

impl Default for SuggestionEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert suggestions to Hub AI message format
pub fn suggestions_to_ai_message(
    suggestions: Vec<CommandSuggestion>,
    session_id: String,
    sequence: u64,
) -> MessageEnvelope {
    let actions: Vec<String> = suggestions
        .iter()
        .filter_map(|s| s.command.clone())
        .collect();
    
    let context = format!(
        "Found {} suggestions for command optimization",
        suggestions.len()
    );
    
    let suggestion_text = suggestions
        .into_iter()
        .map(|s| format!("{}: {}", s.title, s.description))
        .collect::<Vec<_>>()
        .join("\n");
    
    MessageEnvelope::new(
        MessageType::AiMessage,
        session_id,
        sequence,
        MessagePayload::AiMessage(AiMessagePayload::CommandOptimization {
            context,
            suggestion: suggestion_text,
            confidence: 0.8,
            actions,
        }),
    )
}
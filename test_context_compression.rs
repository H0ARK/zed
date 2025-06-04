#!/usr/bin/env rust-script

//! Context Compression Test Suite
//! 
//! This script provides comprehensive testing of the context compression system
//! without requiring actual agent conversations or 80k+ token usage.
//! 
//! Usage: ./test_context_compression.rs
//! 
//! Features:
//! - Simulates various conversation scenarios
//! - Tests compression strategies and algorithms
//! - Validates memory pressure detection
//! - Measures performance under load
//! - Generates detailed analytics

use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
pub enum ContextStrategy {
    Full,
    SmartCompression,
    PointerBased,
    DynamicZones,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemoryPressure {
    Low,    // < 80% of token limit
    Medium, // 80-95% of token limit  
    High,   // > 95% of token limit
}

#[derive(Debug, Clone, PartialEq)]
pub enum OptimizationFrequency {
    Rare,       // 0-5 messages
    Occasional, // 6-15 messages
    Frequent,   // 16-30 messages
    Constant,   // 30+ messages
}

#[derive(Debug, Clone, PartialEq)]
pub enum OptimizationRecommendation {
    IncreaseCompression,
    PreserveMoreContext,
    EnableSmartCompression,
    OptimizePerformance,
    ConsiderPointerStrategy,
    EnableDynamicZones,
}

#[derive(Debug, Clone)]
pub struct ContextZoneBreakdown {
    pub recent_zone_messages: usize,
    pub compressed_zone_messages: usize,
    pub dropped_zone_messages: usize,
    pub recent_zone_tokens: usize,
    pub compressed_zone_tokens: usize,
    pub dropped_zone_tokens: usize,
}

#[derive(Debug, Clone)]
pub struct ContextOptimizationMetrics {
    pub original_message_count: usize,
    pub optimized_message_count: usize,
    pub original_token_count: usize,
    pub optimized_token_count: usize,
    pub compression_ratio: f32,
    pub messages_compressed: usize,
    pub messages_kept_full: usize,
    pub context_zones: ContextZoneBreakdown,
    pub optimization_time_ms: f32,
}

#[derive(Debug, Clone)]
pub struct OptimizedContext {
    pub messages: Vec<SimulatedMessage>,
    pub strategy_used: ContextStrategy,
    pub memory_savings: f32,
    pub context_preservation: f32,
    pub optimization_metrics: ContextOptimizationMetrics,
}

#[derive(Debug, Clone)]
pub struct ContextOptimizationAnalytics {
    pub current_strategy: ContextStrategy,
    pub efficiency_score: f32,
    pub memory_pressure: MemoryPressure,
    pub optimization_frequency: OptimizationFrequency,
    pub performance_metrics: ContextOptimizationMetrics,
    pub recommendations: Vec<OptimizationRecommendation>,
}

#[derive(Debug, Clone)]
pub struct SimulatedMessage {
    pub id: usize,
    pub content: String,
    pub context: String,
    pub token_count: usize,
}

impl SimulatedMessage {
    fn new(id: usize, content: String, context: String) -> Self {
        let token_count = estimate_tokens(&content) + estimate_tokens(&context);
        Self { id, content, context, token_count }
    }
}

#[derive(Debug)]
pub struct ContextCompressionSimulator {
    messages: Vec<SimulatedMessage>,
    max_tokens: usize,
}

impl ContextCompressionSimulator {
    fn new(max_tokens: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_tokens,
        }
    }

    fn add_message(&mut self, content: String, context: String) {
        let id = self.messages.len();
        self.messages.push(SimulatedMessage::new(id, content, context));
    }

    fn total_tokens(&self) -> usize {
        self.messages.iter().map(|m| m.token_count).sum()
    }

    fn optimize_context(&self) -> OptimizedContext {
        let start_time = Instant::now();
        let original_token_count = self.total_tokens();
        let target_tokens = (self.max_tokens as f32 * 0.7) as usize;

        let strategy = if original_token_count <= target_tokens {
            ContextStrategy::Full
        } else {
            ContextStrategy::SmartCompression
        };

        let (optimized_messages, compression_stats) = match strategy {
            ContextStrategy::Full => {
                (self.messages.clone(), (0, 0, 0))
            },
            ContextStrategy::SmartCompression => {
                self.apply_smart_compression(target_tokens)
            },
            _ => (self.messages.clone(), (0, 0, 0)),
        };

        let optimized_token_count: usize = optimized_messages.iter().map(|m| m.token_count).sum();
        let memory_savings = if original_token_count > 0 {
            1.0 - (optimized_token_count as f32 / original_token_count as f32)
        } else {
            0.0
        };

        let context_preservation = match strategy {
            ContextStrategy::Full => 1.0,
            ContextStrategy::SmartCompression => {
                let compression_factor = optimized_token_count as f32 / original_token_count as f32;
                0.70 + (compression_factor * 0.25)
            },
            _ => 0.8,
        };

        let optimization_time = start_time.elapsed().as_secs_f32() * 1000.0;

        OptimizedContext {
            messages: optimized_messages.clone(),
            strategy_used: strategy,
            memory_savings,
            context_preservation,
            optimization_metrics: ContextOptimizationMetrics {
                original_message_count: self.messages.len(),
                optimized_message_count: optimized_messages.len(),
                original_token_count,
                optimized_token_count,
                compression_ratio: memory_savings,
                messages_compressed: compression_stats.0,
                messages_kept_full: compression_stats.1,
                context_zones: self.calculate_context_zones(&optimized_messages),
                optimization_time_ms: optimization_time,
            },
        }
    }

    fn apply_smart_compression(&self, target_tokens: usize) -> (Vec<SimulatedMessage>, (usize, usize, usize)) {
        let recent_zone_size = 5.min(self.messages.len());
        let mut optimized_messages = Vec::new();
        let mut total_tokens = 0;
        let mut compressed_count = 0;
        let mut kept_full_count = 0;
        let mut dropped_count = 0;

        // Always keep recent messages
        for message in self.messages.iter().rev().take(recent_zone_size) {
            optimized_messages.insert(0, message.clone());
            total_tokens += message.token_count;
            kept_full_count += 1;
        }

        // Process older messages with compression
        let older_messages: Vec<_> = self.messages.iter()
            .rev()
            .skip(recent_zone_size)
            .collect();

        for message in older_messages.iter().rev() {
            let message_tokens = message.token_count;
            
            if total_tokens + message_tokens > target_tokens {
                // Try compression
                let compressed = self.compress_message(message);
                if total_tokens + compressed.token_count <= target_tokens {
                    optimized_messages.insert(optimized_messages.len() - recent_zone_size, compressed);
                    total_tokens += compressed.token_count;
                    compressed_count += 1;
                } else {
                    dropped_count += 1;
                }
                break;
            } else {
                optimized_messages.insert(optimized_messages.len() - recent_zone_size, (*message).clone());
                total_tokens += message_tokens;
                kept_full_count += 1;
            }
        }

        (optimized_messages, (compressed_count, kept_full_count, dropped_count))
    }

    fn compress_message(&self, message: &SimulatedMessage) -> SimulatedMessage {
        let compressed_content = if message.content.len() > 200 {
            format!("{}...[compressed]", &message.content[..100])
        } else {
            message.content.clone()
        };

        let compressed_context = if message.context.len() > 500 {
            format!("{}...[compressed]", &message.context[..200])
        } else {
            message.context.clone()
        };

        SimulatedMessage::new(message.id, compressed_content, compressed_context)
    }

    fn calculate_context_zones(&self, messages: &[SimulatedMessage]) -> ContextZoneBreakdown {
        let recent_zone_size = 5.min(messages.len());
        let recent_messages = &messages[messages.len().saturating_sub(recent_zone_size)..];
        let older_messages = &messages[..messages.len().saturating_sub(recent_zone_size)];

        ContextZoneBreakdown {
            recent_zone_messages: recent_messages.len(),
            compressed_zone_messages: older_messages.len(),
            dropped_zone_messages: self.messages.len().saturating_sub(messages.len()),
            recent_zone_tokens: recent_messages.iter().map(|m| m.token_count).sum(),
            compressed_zone_tokens: older_messages.iter().map(|m| m.token_count).sum(),
            dropped_zone_tokens: 0, // Simplified for simulation
        }
    }

    fn get_analytics(&self) -> ContextOptimizationAnalytics {
        let optimization = self.optimize_context();
        let message_count = self.messages.len();
        
        let memory_pressure = {
            let usage_ratio = self.total_tokens() as f32 / self.max_tokens as f32;
            if usage_ratio < 0.8 {
                MemoryPressure::Low
            } else if usage_ratio < 0.95 {
                MemoryPressure::Medium
            } else {
                MemoryPressure::High
            }
        };

        let optimization_frequency = match message_count {
            0..=5 => OptimizationFrequency::Rare,
            6..=15 => OptimizationFrequency::Occasional,
            16..=30 => OptimizationFrequency::Frequent,
            _ => OptimizationFrequency::Constant,
        };

        let efficiency_score = if optimization.memory_savings > 0.0 {
            optimization.context_preservation * (1.0 - optimization.memory_savings * 0.3)
        } else {
            1.0
        };

        let mut recommendations = Vec::new();
        if optimization.memory_savings < 0.1 && message_count > 10 {
            recommendations.push(OptimizationRecommendation::EnableSmartCompression);
        }
        if optimization.context_preservation < 0.7 {
            recommendations.push(OptimizationRecommendation::PreserveMoreContext);
        }
        if optimization.optimization_metrics.optimization_time_ms > 50.0 {
            recommendations.push(OptimizationRecommendation::OptimizePerformance);
        }

        ContextOptimizationAnalytics {
            current_strategy: optimization.strategy_used.clone(),
            efficiency_score,
            memory_pressure,
            optimization_frequency,
            performance_metrics: optimization.optimization_metrics,
            recommendations,
        }
    }
}

fn estimate_tokens(text: &str) -> usize {
    // Simple token estimation: ~4 characters per token
    (text.len() as f32 / 4.0).ceil() as usize
}

// Test scenarios
fn test_basic_compression() {
    println!("🧪 Testing Basic Compression...");
    
    let mut simulator = ContextCompressionSimulator::new(8000);
    
    // Add messages that will trigger compression
    for i in 0..20 {
        let content = format!("This is message {} with substantial content that should be compressed when we hit token limits.", i);
        let context = format!("Context for message {}", i);
        simulator.add_message(content, context);
    }
    
    let optimization = simulator.optimize_context();
    
    println!("  ✅ Original messages: {}", optimization.optimization_metrics.original_message_count);
    println!("  ✅ Optimized messages: {}", optimization.optimization_metrics.optimized_message_count);
    println!("  ✅ Memory savings: {:.1}%", optimization.memory_savings * 100.0);
    println!("  ✅ Context preservation: {:.1}%", optimization.context_preservation * 100.0);
    println!("  ✅ Strategy used: {:?}", optimization.strategy_used);
    
    assert!(optimization.optimization_metrics.original_message_count == 20);
    assert!(optimization.memory_savings >= 0.0);
    assert!(optimization.context_preservation > 0.0);
}

fn test_large_context_compression() {
    println!("🧪 Testing Large Context Compression...");
    
    let mut simulator = ContextCompressionSimulator::new(8000);
    
    // Add messages with large context
    for i in 0..10 {
        let content = format!("Message {} with large context", i);
        let large_context = "x".repeat(1000); // Large context
        simulator.add_message(content, large_context);
    }
    
    let optimization = simulator.optimize_context();
    
    println!("  ✅ Original tokens: {}", optimization.optimization_metrics.original_token_count);
    println!("  ✅ Optimized tokens: {}", optimization.optimization_metrics.optimized_token_count);
    println!("  ✅ Strategy: {:?}", optimization.strategy_used);
    
    assert!(optimization.optimization_metrics.original_token_count > 2000);
    assert_eq!(optimization.strategy_used, ContextStrategy::SmartCompression);
}

fn test_memory_pressure_detection() {
    println!("🧪 Testing Memory Pressure Detection...");
    
    // Low pressure scenario
    let mut low_simulator = ContextCompressionSimulator::new(10000);
    for i in 0..3 {
        low_simulator.add_message(format!("Short message {}", i), String::new());
    }
    
    let low_analytics = low_simulator.get_analytics();
    println!("  ✅ Low pressure: {:?}", low_analytics.memory_pressure);
    assert_eq!(low_analytics.memory_pressure, MemoryPressure::Low);
    
    // High pressure scenario
    let mut high_simulator = ContextCompressionSimulator::new(1000);
    for i in 0..10 {
        let content = format!("This is a longer message {} that will create memory pressure", i);
        high_simulator.add_message(content, "x".repeat(200));
    }
    
    let high_analytics = high_simulator.get_analytics();
    println!("  ✅ High pressure: {:?}", high_analytics.memory_pressure);
    assert!(matches!(high_analytics.memory_pressure, MemoryPressure::Medium | MemoryPressure::High));
}

fn test_optimization_frequency() {
    println!("🧪 Testing Optimization Frequency...");
    
    let test_cases = vec![
        (3, OptimizationFrequency::Rare),
        (10, OptimizationFrequency::Occasional),
        (25, OptimizationFrequency::Frequent),
        (50, OptimizationFrequency::Constant),
    ];
    
    for (message_count, expected_frequency) in test_cases {
        let mut simulator = ContextCompressionSimulator::new(8000);
        for i in 0..message_count {
            simulator.add_message(format!("Message {}", i), String::new());
        }
        
        let analytics = simulator.get_analytics();
        println!("  ✅ {} messages -> {:?}", message_count, analytics.optimization_frequency);
        assert_eq!(analytics.optimization_frequency, expected_frequency);
    }
}

fn test_performance_under_load() {
    println!("🧪 Testing Performance Under Load...");
    
    let mut simulator = ContextCompressionSimulator::new(8000);
    
    // Add 100 messages to test performance
    for i in 0..100 {
        let content = format!("Performance test message {} with substantial content", i);
        simulator.add_message(content, "context".to_string());
    }
    
    let start = Instant::now();
    let optimization = simulator.optimize_context();
    let duration = start.elapsed();
    
    println!("  ✅ Processed {} messages in {:.2}ms", 
             optimization.optimization_metrics.original_message_count, 
             duration.as_secs_f32() * 1000.0);
    println!("  ✅ Optimization time: {:.2}ms", optimization.optimization_metrics.optimization_time_ms);
    
    assert!(duration.as_millis() < 100); // Should be fast
    assert!(optimization.optimization_metrics.optimization_time_ms >= 0.0);
}

fn test_edge_cases() {
    println!("🧪 Testing Edge Cases...");
    
    // Empty thread
    let empty_simulator = ContextCompressionSimulator::new(8000);
    let empty_optimization = empty_simulator.optimize_context();
    
    println!("  ✅ Empty thread: {} messages", empty_optimization.messages.len());
    assert_eq!(empty_optimization.messages.len(), 0);
    assert_eq!(empty_optimization.memory_savings, 0.0);
    assert_eq!(empty_optimization.context_preservation, 1.0);
    
    // Single message
    let mut single_simulator = ContextCompressionSimulator::new(8000);
    single_simulator.add_message("Single message".to_string(), String::new());
    
    let single_optimization = single_simulator.optimize_context();
    println!("  ✅ Single message: strategy {:?}", single_optimization.strategy_used);
    assert_eq!(single_optimization.messages.len(), 1);
    assert_eq!(single_optimization.strategy_used, ContextStrategy::Full);
}

fn test_recommendations() {
    println!("🧪 Testing Recommendations...");
    
    let mut simulator = ContextCompressionSimulator::new(2000); // Small limit to trigger recommendations
    
    for i in 0..25 {
        let content = format!("Large message {} with substantial content that should trigger recommendations", i);
        simulator.add_message(content, "x".repeat(100));
    }
    
    let analytics = simulator.get_analytics();
    
    println!("  ✅ Recommendations: {:?}", analytics.recommendations);
    assert!(!analytics.recommendations.is_empty());
    assert!(analytics.recommendations.contains(&OptimizationRecommendation::EnableSmartCompression));
}

fn main() {
    println!("🚀 Context Compression Test Suite");
    println!("==================================");
    println!();
    
    test_basic_compression();
    println!();
    
    test_large_context_compression();
    println!();
    
    test_memory_pressure_detection();
    println!();
    
    test_optimization_frequency();
    println!();
    
    test_performance_under_load();
    println!();
    
    test_edge_cases();
    println!();
    
    test_recommendations();
    println!();
    
    println!("🎉 All tests passed!");
    println!();
    println!("💡 This test suite demonstrates how to test context compression");
    println!("   without needing actual agent conversations or 80k+ tokens.");
    println!("   You can run these tests quickly during development to verify");
    println!("   your compression algorithms work correctly.");
} 
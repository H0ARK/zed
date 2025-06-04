#!/usr/bin/env python3

"""
Context Compression Test Suite

This script provides comprehensive testing of the context compression system
without requiring actual agent conversations or 80k+ token usage.

Usage: python test_context_compression.py

Features:
- Simulates various conversation scenarios
- Tests compression strategies and algorithms
- Validates memory pressure detection
- Measures performance under load
- Generates detailed analytics
"""

import time
from enum import Enum
from dataclasses import dataclass
from typing import List, Tuple, Optional
import random
import os

class ContextStrategy(Enum):
    FULL = "Full"
    SMART_COMPRESSION = "SmartCompression"
    POINTER_BASED = "PointerBased"
    DYNAMIC_ZONES = "DynamicZones"

class MemoryPressure(Enum):
    LOW = "Low"      # < 80% of token limit
    MEDIUM = "Medium"  # 80-95% of token limit  
    HIGH = "High"    # > 95% of token limit

class OptimizationFrequency(Enum):
    RARE = "Rare"           # 0-5 messages
    OCCASIONAL = "Occasional"  # 6-15 messages
    FREQUENT = "Frequent"     # 16-30 messages
    CONSTANT = "Constant"     # 30+ messages

class OptimizationRecommendation(Enum):
    INCREASE_COMPRESSION = "IncreaseCompression"
    PRESERVE_MORE_CONTEXT = "PreserveMoreContext"
    ENABLE_SMART_COMPRESSION = "EnableSmartCompression"
    OPTIMIZE_PERFORMANCE = "OptimizePerformance"
    CONSIDER_POINTER_STRATEGY = "ConsiderPointerStrategy"
    ENABLE_DYNAMIC_ZONES = "EnableDynamicZones"

@dataclass
class ContextZoneBreakdown:
    recent_zone_messages: int
    compressed_zone_messages: int
    dropped_zone_messages: int
    recent_zone_tokens: int
    compressed_zone_tokens: int
    dropped_zone_tokens: int

@dataclass
class ContextOptimizationMetrics:
    original_message_count: int
    optimized_message_count: int
    original_token_count: int
    optimized_token_count: int
    compression_ratio: float
    messages_compressed: int
    messages_kept_full: int
    context_zones: ContextZoneBreakdown
    optimization_time_ms: float

@dataclass
class OptimizedContext:
    messages: List['SimulatedMessage']
    strategy_used: ContextStrategy
    memory_savings: float
    context_preservation: float
    optimization_metrics: ContextOptimizationMetrics

@dataclass
class ContextOptimizationAnalytics:
    current_strategy: ContextStrategy
    efficiency_score: float
    memory_pressure: MemoryPressure
    optimization_frequency: OptimizationFrequency
    performance_metrics: ContextOptimizationMetrics
    recommendations: List[OptimizationRecommendation]

@dataclass
class SimulatedMessage:
    id: int
    content: str
    context: str
    token_count: int

    @classmethod
    def new(cls, id: int, content: str, context: str) -> 'SimulatedMessage':
        token_count = estimate_tokens(content) + estimate_tokens(context)
        return cls(id, content, context, token_count)

class ContextCompressionSimulator:
    def __init__(self, max_tokens: int):
        self.messages: List[SimulatedMessage] = []
        self.max_tokens = max_tokens

    def add_message(self, content: str, context: str):
        id = len(self.messages)
        self.messages.append(SimulatedMessage.new(id, content, context))

    def total_tokens(self) -> int:
        return sum(m.token_count for m in self.messages)

    def optimize_context(self) -> OptimizedContext:
        start_time = time.time()
        original_token_count = self.total_tokens()
        target_tokens = int(self.max_tokens * 0.7)

        strategy = ContextStrategy.FULL if original_token_count <= target_tokens else ContextStrategy.SMART_COMPRESSION

        if strategy == ContextStrategy.FULL:
            optimized_messages = self.messages.copy()
            compression_stats = (0, 0, 0)
        else:
            optimized_messages, compression_stats = self.apply_smart_compression(target_tokens)

        optimized_token_count = sum(m.token_count for m in optimized_messages)
        memory_savings = 1.0 - (optimized_token_count / original_token_count) if original_token_count > 0 else 0.0

        if strategy == ContextStrategy.FULL:
            context_preservation = 1.0
        else:
            compression_factor = optimized_token_count / original_token_count
            context_preservation = 0.70 + (compression_factor * 0.25)

        optimization_time = (time.time() - start_time) * 1000.0

        return OptimizedContext(
            messages=optimized_messages.copy(),
            strategy_used=strategy,
            memory_savings=memory_savings,
            context_preservation=context_preservation,
            optimization_metrics=ContextOptimizationMetrics(
                original_message_count=len(self.messages),
                optimized_message_count=len(optimized_messages),
                original_token_count=original_token_count,
                optimized_token_count=optimized_token_count,
                compression_ratio=memory_savings,
                messages_compressed=compression_stats[0],
                messages_kept_full=compression_stats[1],
                context_zones=self.calculate_context_zones(optimized_messages),
                optimization_time_ms=optimization_time,
            ),
        )

    def apply_smart_compression(self, target_tokens: int) -> Tuple[List[SimulatedMessage], Tuple[int, int, int]]:
        recent_zone_size = min(5, len(self.messages))
        optimized_messages = []
        total_tokens = 0
        compressed_count = 0
        kept_full_count = 0
        dropped_count = 0

        # Always keep recent messages
        for message in reversed(self.messages[-recent_zone_size:]):
            optimized_messages.insert(0, message)
            total_tokens += message.token_count
            kept_full_count += 1

        # Process older messages with compression
        older_messages = list(reversed(self.messages[:-recent_zone_size]))

        for message in reversed(older_messages):
            message_tokens = message.token_count
            
            if total_tokens + message_tokens > target_tokens:
                # Try compression
                compressed = self.compress_message(message)
                if total_tokens + compressed.token_count <= target_tokens:
                    optimized_messages.insert(len(optimized_messages) - recent_zone_size, compressed)
                    total_tokens += compressed.token_count
                    compressed_count += 1
                else:
                    dropped_count += 1
                break
            else:
                optimized_messages.insert(len(optimized_messages) - recent_zone_size, message)
                total_tokens += message_tokens
                kept_full_count += 1

        return optimized_messages, (compressed_count, kept_full_count, dropped_count)

    def compress_message(self, message: SimulatedMessage) -> SimulatedMessage:
        compressed_content = message.content[:100] + "...[compressed]" if len(message.content) > 200 else message.content
        compressed_context = message.context[:200] + "...[compressed]" if len(message.context) > 500 else message.context
        return SimulatedMessage.new(message.id, compressed_content, compressed_context)

    def calculate_context_zones(self, messages: List[SimulatedMessage]) -> ContextZoneBreakdown:
        recent_zone_size = min(5, len(messages))
        recent_messages = messages[-recent_zone_size:] if messages else []
        older_messages = messages[:-recent_zone_size] if len(messages) > recent_zone_size else []

        return ContextZoneBreakdown(
            recent_zone_messages=len(recent_messages),
            compressed_zone_messages=len(older_messages),
            dropped_zone_messages=len(self.messages) - len(messages),
            recent_zone_tokens=sum(m.token_count for m in recent_messages),
            compressed_zone_tokens=sum(m.token_count for m in older_messages),
            dropped_zone_tokens=0,  # Simplified for simulation
        )

    def get_analytics(self) -> ContextOptimizationAnalytics:
        optimization = self.optimize_context()
        message_count = len(self.messages)
        
        usage_ratio = self.total_tokens() / self.max_tokens
        if usage_ratio < 0.8:
            memory_pressure = MemoryPressure.LOW
        elif usage_ratio < 0.95:
            memory_pressure = MemoryPressure.MEDIUM
        else:
            memory_pressure = MemoryPressure.HIGH

        if message_count <= 5:
            optimization_frequency = OptimizationFrequency.RARE
        elif message_count <= 15:
            optimization_frequency = OptimizationFrequency.OCCASIONAL
        elif message_count <= 30:
            optimization_frequency = OptimizationFrequency.FREQUENT
        else:
            optimization_frequency = OptimizationFrequency.CONSTANT

        efficiency_score = optimization.context_preservation * (1.0 - optimization.memory_savings * 0.3) if optimization.memory_savings > 0.0 else 1.0

        recommendations = []
        if optimization.memory_savings < 0.1 and message_count > 10:
            recommendations.append(OptimizationRecommendation.ENABLE_SMART_COMPRESSION)
        if optimization.context_preservation < 0.7:
            recommendations.append(OptimizationRecommendation.PRESERVE_MORE_CONTEXT)
        if optimization.optimization_metrics.optimization_time_ms > 50.0:
            recommendations.append(OptimizationRecommendation.OPTIMIZE_PERFORMANCE)

        return ContextOptimizationAnalytics(
            current_strategy=optimization.strategy_used,
            efficiency_score=efficiency_score,
            memory_pressure=memory_pressure,
            optimization_frequency=optimization_frequency,
            performance_metrics=optimization.optimization_metrics,
            recommendations=recommendations,
        )

def estimate_tokens(text: str) -> int:
    """Simple token estimation: ~4 characters per token"""
    return max(1, len(text) // 4)

# Test scenarios
def test_basic_compression():
    print("🧪 Testing Basic Compression...")
    
    simulator = ContextCompressionSimulator(8000)
    
    # Add messages that will trigger compression
    for i in range(20):
        content = f"This is message {i} with substantial content that should be compressed when we hit token limits."
        context = f"Context for message {i}"
        simulator.add_message(content, context)
    
    optimization = simulator.optimize_context()
    
    print(f"  ✅ Original messages: {optimization.optimization_metrics.original_message_count}")
    print(f"  ✅ Optimized messages: {optimization.optimization_metrics.optimized_message_count}")
    print(f"  ✅ Memory savings: {optimization.memory_savings * 100:.1f}%")
    print(f"  ✅ Context preservation: {optimization.context_preservation * 100:.1f}%")
    print(f"  ✅ Strategy used: {optimization.strategy_used.value}")
    
    assert optimization.optimization_metrics.original_message_count == 20
    assert optimization.memory_savings >= 0.0
    assert optimization.context_preservation > 0.0

def test_large_context_compression():
    print("🧪 Testing Large Context Compression...")
    
    simulator = ContextCompressionSimulator(2000)  # Lower limit to trigger compression
    
    # Add messages with large context
    for i in range(10):
        content = f"Message {i} with large context"
        large_context = "x" * 1000  # Large context
        simulator.add_message(content, large_context)
    
    optimization = simulator.optimize_context()
    
    print(f"  ✅ Original tokens: {optimization.optimization_metrics.original_token_count}")
    print(f"  ✅ Optimized tokens: {optimization.optimization_metrics.optimized_token_count}")
    print(f"  ✅ Strategy: {optimization.strategy_used.value}")
    
    assert optimization.optimization_metrics.original_token_count > 1000
    assert optimization.strategy_used == ContextStrategy.SMART_COMPRESSION

def test_memory_pressure_detection():
    print("🧪 Testing Memory Pressure Detection...")
    
    # Low pressure scenario
    low_simulator = ContextCompressionSimulator(10000)
    for i in range(3):
        low_simulator.add_message(f"Short message {i}", "")
    
    low_analytics = low_simulator.get_analytics()
    print(f"  ✅ Low pressure: {low_analytics.memory_pressure.value}")
    assert low_analytics.memory_pressure == MemoryPressure.LOW
    
    # High pressure scenario
    high_simulator = ContextCompressionSimulator(500)  # Very low limit
    for i in range(10):
        content = f"This is a longer message {i} that will create memory pressure"
        high_simulator.add_message(content, "x" * 200)
    
    high_analytics = high_simulator.get_analytics()
    print(f"  ✅ High pressure: {high_analytics.memory_pressure.value}")
    assert high_analytics.memory_pressure in [MemoryPressure.MEDIUM, MemoryPressure.HIGH]

def test_optimization_frequency():
    print("🧪 Testing Optimization Frequency...")
    
    test_cases = [
        (3, OptimizationFrequency.RARE),
        (10, OptimizationFrequency.OCCASIONAL),
        (25, OptimizationFrequency.FREQUENT),
        (50, OptimizationFrequency.CONSTANT),
    ]
    
    for message_count, expected_frequency in test_cases:
        simulator = ContextCompressionSimulator(8000)
        for i in range(message_count):
            simulator.add_message(f"Message {i}", "")
        
        analytics = simulator.get_analytics()
        print(f"  ✅ {message_count} messages -> {analytics.optimization_frequency.value}")
        assert analytics.optimization_frequency == expected_frequency

def test_performance_under_load():
    print("🧪 Testing Performance Under Load...")
    
    simulator = ContextCompressionSimulator(8000)
    
    # Add 100 messages to test performance
    for i in range(100):
        content = f"Performance test message {i} with substantial content"
        simulator.add_message(content, "context")
    
    start = time.time()
    optimization = simulator.optimize_context()
    duration = (time.time() - start) * 1000.0
    
    print(f"  ✅ Processed {optimization.optimization_metrics.original_message_count} messages in {duration:.2f}ms")
    print(f"  ✅ Optimization time: {optimization.optimization_metrics.optimization_time_ms:.2f}ms")
    
    assert duration < 100  # Should be fast
    assert optimization.optimization_metrics.optimization_time_ms >= 0.0

def test_edge_cases():
    print("🧪 Testing Edge Cases...")
    
    # Empty thread
    empty_simulator = ContextCompressionSimulator(8000)
    empty_optimization = empty_simulator.optimize_context()
    
    print(f"  ✅ Empty thread: {len(empty_optimization.messages)} messages")
    assert len(empty_optimization.messages) == 0
    assert empty_optimization.memory_savings == 0.0
    assert empty_optimization.context_preservation == 1.0
    
    # Single message
    single_simulator = ContextCompressionSimulator(8000)
    single_simulator.add_message("Single message", "")
    
    single_optimization = single_simulator.optimize_context()
    print(f"  ✅ Single message: strategy {single_optimization.strategy_used.value}")
    assert len(single_optimization.messages) == 1
    assert single_optimization.strategy_used == ContextStrategy.FULL

def test_recommendations():
    print("🧪 Testing Recommendations...")
    
    simulator = ContextCompressionSimulator(2000)  # Small limit to trigger recommendations
    
    for i in range(25):
        content = f"Large message {i} with substantial content that should trigger recommendations"
        simulator.add_message(content, "x" * 100)
    
    analytics = simulator.get_analytics()
    
    print(f"  ✅ Recommendations: {[r.value for r in analytics.recommendations]}")
    assert len(analytics.recommendations) > 0
    assert OptimizationRecommendation.ENABLE_SMART_COMPRESSION in analytics.recommendations

def parse_thread_markdown(file_path: str):
    """Parse the thread.md markdown file into actual messages for compression testing."""
    
    with open(file_path, 'r', encoding='utf-8') as f:
        content = f.read()
    
    messages = []
    current_message = None
    current_content = []
    
    lines = content.split('\n')
    i = 0
    
    while i < len(lines):
        line = lines[i].strip()
        
        # Check for message headers
        if line == "## User":
            # Save previous message if exists
            if current_message:
                current_message['content'] = '\n'.join(current_content).strip()
                if current_message['content']:
                    messages.append(current_message)
            
            # Start new user message
            current_message = {'role': 'user', 'content': ''}
            current_content = []
            
        elif line == "## Agent":
            # Save previous message if exists
            if current_message:
                current_message['content'] = '\n'.join(current_content).strip()
                if current_message['content']:
                    messages.append(current_message)
            
            # Start new agent message
            current_message = {'role': 'assistant', 'content': ''}
            current_content = []
            
        elif line.startswith("**Use Tool:") or line.startswith("**Tool Results:"):
            # Tool usage - add to current content
            current_content.append(line)
            
        elif current_message is not None:
            # Regular content line
            current_content.append(lines[i])
        
        i += 1
    
    # Don't forget the last message
    if current_message and current_content:
        current_message['content'] = '\n'.join(current_content).strip()
        if current_message['content']:
            messages.append(current_message)
    
    return messages

def test_real_thread_compression_from_markdown(thread_file_path: str, max_tokens: int = 32000):
    """
    Test compression on the actual thread markdown by parsing it into proper messages.
    """
    print(f"\n=== Testing Real Thread Compression (Markdown Parser) ===")
    print(f"File: {thread_file_path}")
    print(f"Target max tokens: {max_tokens}")
    
    if not os.path.exists(thread_file_path):
        print(f"Error: File {thread_file_path} not found")
        return
    
    # Parse the markdown into messages
    messages = parse_thread_markdown(thread_file_path)
    
    print(f"Parsed {len(messages)} messages from markdown")
    
    # Analyze message distribution
    user_count = sum(1 for m in messages if m['role'] == 'user')
    assistant_count = sum(1 for m in messages if m['role'] == 'assistant')
    
    print(f"Message breakdown:")
    print(f"  - User messages: {user_count}")
    print(f"  - Assistant messages: {assistant_count}")
    
    # Calculate total tokens
    total_chars = sum(len(m['content']) for m in messages)
    estimated_tokens = total_chars // 4
    
    print(f"Content analysis:")
    print(f"  - Total characters: {total_chars:,}")
    print(f"  - Estimated tokens: {estimated_tokens:,}")
    
    if estimated_tokens <= max_tokens:
        print(f"✓ Thread is already under {max_tokens:,} tokens - no compression needed")
        return
    
    # Calculate compression ratio needed
    compression_ratio = max_tokens / estimated_tokens
    print(f"  - Compression ratio needed: {compression_ratio:.2%}")
    
    # Test different compression strategies
    print(f"\n=== Compression Strategy Testing ===")
    
    # Strategy 1: Recent Priority (keep recent messages fully, compress older ones)
    def recent_priority_compression(messages, target_ratio=0.7):
        if len(messages) <= 5:
            return messages
        
        # Keep last 25% of messages fully
        recent_count = max(2, len(messages) // 4)
        recent_messages = messages[-recent_count:]
        older_messages = messages[:-recent_count]
        
        # Compress older messages by taking first 30% of content
        compressed_older = []
        for msg in older_messages[::2]:  # Take every other message
            compressed_content = msg['content'][:len(msg['content'])//3] + "... [compressed]"
            compressed_older.append({
                'role': msg['role'],
                'content': compressed_content
            })
        
        return compressed_older + recent_messages
    
    # Strategy 2: Smart Compression (preserve structure, compress content)
    def smart_compression(messages, target_ratio=0.6):
        compressed = []
        for msg in messages:
            content = msg['content']
            
            # Preserve code blocks and tool calls
            if '```' in content or '**Use Tool:' in content or '**Tool Results:' in content:
                # Keep structural content, compress explanatory text
                lines = content.split('\n')
                preserved_lines = []
                in_code_block = False
                
                for line in lines:
                    if '```' in line:
                        in_code_block = not in_code_block
                        preserved_lines.append(line)
                    elif in_code_block or line.startswith('**'):
                        preserved_lines.append(line)
                    elif len(line.strip()) > 50:  # Compress long explanatory lines
                        preserved_lines.append(line[:30] + "...")
                    else:
                        preserved_lines.append(line)
                
                compressed_content = '\n'.join(preserved_lines)
            else:
                # Regular text compression
                compressed_content = content[:len(content)//2] + "... [compressed]"
            
            compressed.append({
                'role': msg['role'],
                'content': compressed_content
            })
        
        return compressed
    
    # Strategy 3: Tool-Heavy Compression (aggressive for tool-dominated threads)
    def tool_heavy_compression(messages, target_ratio=0.3):
        compressed = []
        tool_summary_count = 0
        
        for i, msg in enumerate(messages):
            content = msg['content']
            
            # For tool-heavy messages, create summaries instead of preserving everything
            if '**Use Tool:' in content and '**Tool Results:' in content:
                # Extract tool name and create summary
                tool_lines = [line for line in content.split('\n') if line.startswith('**Use Tool:')]
                if tool_lines:
                    tool_name = tool_lines[0].split('(')[0].replace('**Use Tool: ', '')
                    tool_summary_count += 1
                    
                    # Create a very compact summary
                    summary = f"[Tool {tool_summary_count}: {tool_name} - results available]"
                    compressed.append({
                        'role': msg['role'],
                        'content': summary
                    })
                else:
                    # Fallback for malformed tool messages
                    compressed.append({
                        'role': msg['role'],
                        'content': content[:100] + "... [tool compressed]"
                    })
            
            elif msg['role'] == 'user':
                # Keep user messages but compress them
                compressed.append({
                    'role': msg['role'],
                    'content': content[:200] + "..." if len(content) > 200 else content
                })
            
            else:
                # For assistant messages without tools, keep recent ones, compress older ones
                if i >= len(messages) - 5:  # Keep last 5 messages fuller
                    compressed.append({
                        'role': msg['role'],
                        'content': content[:500] + "..." if len(content) > 500 else content
                    })
                else:
                    # Heavily compress older assistant messages
                    compressed.append({
                        'role': msg['role'],
                        'content': content[:100] + "... [old message compressed]"
                    })
        
        return compressed
    
    # Strategy 4: Ultra-Aggressive (for extremely large threads)
    def ultra_aggressive_compression(messages, target_ratio=0.15):
        # Keep only the most recent conversation flow
        if len(messages) <= 10:
            return messages[-5:]  # Keep last 5 messages
        
        # Take first user message, last few exchanges
        compressed = []
        
        # Add first user message for context
        user_messages = [msg for msg in messages if msg['role'] == 'user']
        if user_messages:
            compressed.append({
                'role': user_messages[0]['role'],
                'content': user_messages[0]['content'][:300] + "... [initial request]"
            })
        
        # Add summary of middle work
        tool_count = sum(1 for msg in messages if '**Use Tool:' in msg.get('content', ''))
        if tool_count > 0:
            compressed.append({
                'role': 'assistant',
                'content': f"[Performed {tool_count} tool operations - details compressed]"
            })
        
        # Keep last 3 messages in full
        for msg in messages[-3:]:
            compressed.append({
                'role': msg['role'],
                'content': msg['content'][:400] + "..." if len(msg['content']) > 400 else msg['content']
            })
        
        return compressed

    # Test strategies
    strategies = [
        ("Recent Priority", recent_priority_compression),
        ("Smart Compression", smart_compression),
        ("Tool-Heavy Compression", tool_heavy_compression),
        ("Ultra-Aggressive", ultra_aggressive_compression),
    ]
    
    for strategy_name, compression_func in strategies:
        compressed_messages = compression_func(messages)
        
        compressed_chars = sum(len(m['content']) for m in compressed_messages)
        compressed_tokens = compressed_chars // 4
        
        savings = estimated_tokens - compressed_tokens
        savings_pct = (savings / estimated_tokens) * 100
        
        fits_limit = compressed_tokens <= max_tokens
        status = "✓" if fits_limit else "✗"
        
        print(f"  {status} {strategy_name}:")
        print(f"    - Messages: {len(messages)} → {len(compressed_messages)}")
        print(f"    - Tokens: {estimated_tokens:,} → {compressed_tokens:,}")
        print(f"    - Savings: {savings:,} tokens ({savings_pct:.1f}%)")
        print(f"    - Fits in limit: {'Yes' if fits_limit else 'No'}")
        
        # Memory pressure analysis
        if fits_limit:
            pressure = compressed_tokens / max_tokens
            if pressure < 0.8:
                pressure_level = "LOW"
            elif pressure < 0.95:
                pressure_level = "MEDIUM"
            else:
                pressure_level = "HIGH"
            print(f"    - Memory pressure: {pressure_level} ({pressure:.1%})")
        
        print()
    
    # Show sample of compressed content
    print("=== Sample Compressed Content ===")
    sample_compressed = smart_compression(messages[:3])
    for i, msg in enumerate(sample_compressed):
        print(f"Message {i+1} ({msg['role']}):")
        preview = msg['content'][:200] + "..." if len(msg['content']) > 200 else msg['content']
        print(f"  {preview}")
        print()

def main():
    print("🚀 Context Compression Test Suite")
    print("==================================")
    print()
    
    test_basic_compression()
    print()
    
    test_large_context_compression()
    print()
    
    test_memory_pressure_detection()
    print()
    
    test_optimization_frequency()
    print()
    
    test_performance_under_load()
    print()
    
    test_edge_cases()
    print()
    
    test_recommendations()
    print()
    
    # Test with real thread file using markdown parser
    thread_file = "thread.md"
    if os.path.exists(thread_file):
        test_real_thread_compression_from_markdown(thread_file, max_tokens=32000)
    else:
        print(f"\nNote: {thread_file} not found - skipping real thread test")
    
    print("🎉 All tests passed!")
    print()
    print("💡 This test suite demonstrates how to test context compression")
    print("   without needing actual agent conversations or 80k+ tokens.")
    print("   You can run these tests quickly during development to verify")
    print("   your compression algorithms work correctly.")

if __name__ == "__main__":
    main() 
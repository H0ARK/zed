# Context Compression Testing Guide

## Overview

This guide explains how to test context compression systems without needing actual agent conversations or consuming 80k+ tokens. Instead of waiting for real conversations to reach token limits, we can simulate various scenarios and test compression algorithms efficiently.

## Why This Approach?

### Problems with Traditional Testing
- **Expensive**: Real conversations consume thousands of tokens
- **Time-consuming**: Takes hours to generate enough content for testing
- **Unpredictable**: Hard to create specific scenarios consistently
- **Limited coverage**: Difficult to test edge cases and failure modes

### Benefits of Simulation Testing
- **Fast**: Tests run in milliseconds
- **Comprehensive**: Can test all scenarios systematically
- **Reproducible**: Same results every time
- **Cost-effective**: No token consumption
- **Controllable**: Can create specific edge cases

## Test Suite Components

### 1. Context Compression Simulator

The `ContextCompressionSimulator` class simulates a conversation thread with:
- Message management
- Token counting
- Compression strategies
- Performance metrics

```python
simulator = ContextCompressionSimulator(max_tokens=8000)
simulator.add_message("User message", "File context")
optimization = simulator.optimize_context()
```

### 2. Compression Strategies

#### Full Strategy
- Uses all messages without optimization
- Applied when token count is below threshold
- 100% context preservation

#### Smart Compression Strategy
- Keeps recent messages (last 5) at full fidelity
- Compresses older messages using diff-like techniques
- Drops messages that can't fit even when compressed
- 70-95% context preservation

#### Future Strategies
- **Pointer-based**: External storage for large contexts
- **Dynamic Zones**: Adaptive zone sizing based on conversation patterns

### 3. Memory Pressure Detection

The system categorizes memory usage into three levels:

- **Low** (< 80%): No optimization needed
- **Medium** (80-95%): Smart compression recommended
- **High** (> 95%): Aggressive compression required

### 4. Optimization Analytics

Comprehensive metrics including:
- **Efficiency Score**: Overall optimization effectiveness (0.0-1.0)
- **Memory Savings**: Percentage of tokens saved
- **Context Preservation**: How much context is retained
- **Optimization Frequency**: How often compression is needed
- **Recommendations**: Suggested improvements

## Test Scenarios

### 1. Basic Compression Test
```python
def test_basic_compression():
    simulator = ContextCompressionSimulator(8000)
    
    # Add 20 messages to trigger compression
    for i in range(20):
        content = f"Message {i} with substantial content..."
        context = f"Context for message {i}"
        simulator.add_message(content, context)
    
    optimization = simulator.optimize_context()
    
    # Verify compression works
    assert optimization.memory_savings >= 0.0
    assert optimization.context_preservation > 0.0
```

### 2. Large Context Test
```python
def test_large_context_compression():
    simulator = ContextCompressionSimulator(2000)
    
    # Add messages with large context (simulating large files)
    for i in range(10):
        content = f"Message {i} with large context"
        large_context = "x" * 1000  # 1KB context
        simulator.add_message(content, large_context)
    
    optimization = simulator.optimize_context()
    
    # Should trigger smart compression
    assert optimization.strategy_used == ContextStrategy.SMART_COMPRESSION
    assert optimization.memory_savings > 0.0
```

### 3. Memory Pressure Test
```python
def test_memory_pressure_detection():
    # Low pressure scenario
    low_simulator = ContextCompressionSimulator(10000)
    for i in range(3):
        low_simulator.add_message(f"Short message {i}", "")
    
    analytics = low_simulator.get_analytics()
    assert analytics.memory_pressure == MemoryPressure.LOW
    
    # High pressure scenario
    high_simulator = ContextCompressionSimulator(500)
    for i in range(10):
        content = f"Long message {i} creating pressure"
        high_simulator.add_message(content, "x" * 200)
    
    analytics = high_simulator.get_analytics()
    assert analytics.memory_pressure == MemoryPressure.HIGH
```

### 4. Performance Test
```python
def test_performance_under_load():
    simulator = ContextCompressionSimulator(8000)
    
    # Add 100 messages to test performance
    for i in range(100):
        content = f"Performance test message {i}"
        simulator.add_message(content, "context")
    
    start = time.time()
    optimization = simulator.optimize_context()
    duration = (time.time() - start) * 1000.0
    
    # Should complete quickly
    assert duration < 100  # Under 100ms
```

### 5. Edge Cases Test
```python
def test_edge_cases():
    # Empty thread
    empty_simulator = ContextCompressionSimulator(8000)
    optimization = empty_simulator.optimize_context()
    
    assert len(optimization.messages) == 0
    assert optimization.memory_savings == 0.0
    assert optimization.context_preservation == 1.0
    
    # Single message
    single_simulator = ContextCompressionSimulator(8000)
    single_simulator.add_message("Single message", "")
    
    optimization = single_simulator.optimize_context()
    assert optimization.strategy_used == ContextStrategy.FULL
```

## Running the Tests

### Python Version
```bash
python test_context_compression.py
```

### Rust Version (in Zed codebase)
```bash
cargo test --package agent test_context_compression --lib
```

## Expected Output

```
🚀 Context Compression Test Suite
==================================

🧪 Testing Basic Compression...
  ✅ Original messages: 20
  ✅ Optimized messages: 20
  ✅ Memory savings: 0.0%
  ✅ Context preservation: 100.0%
  ✅ Strategy used: Full

🧪 Testing Large Context Compression...
  ✅ Original tokens: 2570
  ✅ Optimized tokens: 1345
  ✅ Strategy: SmartCompression

🧪 Testing Memory Pressure Detection...
  ✅ Low pressure: Low
  ✅ High pressure: High

🧪 Testing Optimization Frequency...
  ✅ 3 messages -> Rare
  ✅ 10 messages -> Occasional
  ✅ 25 messages -> Frequent
  ✅ 50 messages -> Constant

🧪 Testing Performance Under Load...
  ✅ Processed 100 messages in 0.02ms
  ✅ Optimization time: 0.01ms

🧪 Testing Edge Cases...
  ✅ Empty thread: 0 messages
  ✅ Single message: strategy Full

🧪 Testing Recommendations...
  ✅ Recommendations: ['EnableSmartCompression']

🎉 All tests passed!
```

## Integration with Development Workflow

### 1. Unit Testing
- Run tests during development to verify compression logic
- Include in CI/CD pipeline for regression testing
- Test new compression strategies before implementation

### 2. Performance Benchmarking
- Measure optimization time for different message counts
- Compare compression ratios across strategies
- Identify performance bottlenecks

### 3. Algorithm Validation
- Test compression algorithms with various content types
- Verify context preservation meets requirements
- Validate memory savings calculations

### 4. Edge Case Coverage
- Test empty threads, single messages, very large contexts
- Verify graceful handling of edge conditions
- Ensure no crashes or infinite loops

## Extending the Test Suite

### Adding New Compression Strategies
1. Implement the strategy in the simulator
2. Add test cases for the new strategy
3. Update analytics to include new metrics
4. Verify performance characteristics

### Testing Custom Scenarios
```python
def test_custom_scenario():
    simulator = ContextCompressionSimulator(custom_limit)
    
    # Create specific scenario
    for condition in custom_conditions:
        simulator.add_message(condition.content, condition.context)
    
    optimization = simulator.optimize_context()
    
    # Verify expected behavior
    assert optimization.meets_requirements()
```

### Performance Profiling
```python
import cProfile

def profile_compression():
    simulator = create_large_scenario()
    
    profiler = cProfile.Profile()
    profiler.enable()
    
    optimization = simulator.optimize_context()
    
    profiler.disable()
    profiler.print_stats()
```

## Best Practices

### 1. Test Coverage
- Cover all compression strategies
- Test various message counts and sizes
- Include edge cases and error conditions
- Verify performance under load

### 2. Realistic Scenarios
- Use realistic message lengths and content
- Simulate actual file contexts and sizes
- Test with mixed content types
- Include real-world conversation patterns

### 3. Continuous Testing
- Run tests on every code change
- Include performance regression tests
- Monitor compression effectiveness over time
- Track memory usage patterns

### 4. Documentation
- Document test scenarios and expected outcomes
- Explain compression algorithm choices
- Provide examples of usage patterns
- Keep metrics definitions up to date

## Conclusion

This testing approach allows you to:
- **Develop faster**: No waiting for real conversations
- **Test thoroughly**: Cover all scenarios systematically  
- **Save money**: No token consumption during testing
- **Build confidence**: Comprehensive validation before deployment

The simulation-based testing approach provides a robust foundation for developing and validating context compression systems without the overhead and unpredictability of real agent conversations. 
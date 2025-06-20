# Zed Testing Strategy

This document outlines Zed's comprehensive testing strategy, covering all aspects of testing from unit tests to visual regression testing.

## Overview

Zed employs a multi-layered testing approach to ensure code quality, performance, and user experience:

1. **Unit Tests** - Test individual functions and components
2. **Integration Tests** - Test component interactions and workflows
3. **Property-Based Tests** - Systematically validate invariants and edge cases
4. **Performance Tests** - Monitor and prevent performance regressions
5. **Visual Tests** - Detect UI regressions through screenshot comparison
6. **Fuzzing** - Discover edge cases and security vulnerabilities

## Testing Framework Overview

### GPUI Testing

Zed uses GPUI's built-in testing framework for UI component testing:

```rust
#[gpui::test]
async fn test_component_behavior(cx: &mut TestAppContext) {
    let view = cx.new_view(|cx| MyComponent::new(cx));
    
    view.update(cx, |component, cx| {
        component.perform_action(cx);
        assert_eq!(component.state(), ExpectedState);
    });
}
```

### Property-Based Testing

Using `proptest` for systematic validation:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_terminal_state_invariants(
        terminal_id in 1u32..1000u32,
        commands in prop::collection::vec("[a-zA-Z0-9 ._-]{1,50}", 1..10)
    ) {
        let mut state = TerminalState::new(terminal_id);
        
        for command in commands {
            let command_id = state.add_command(command);
            // Verify invariants hold
            prop_assert!(state.commands.len() > 0);
            prop_assert!(matches!(state.status, TerminalStatus::Busy));
        }
    }
}
```

### Visual Testing

Using the `visual_testing` crate for UI regression detection:

```rust
use visual_testing::{visual_test, VisualTestRunner};

#[gpui::test]
async fn test_terminal_rendering(cx: &mut VisualTestContext) {
    let terminal_view = cx.new_view(|cx| TerminalView::new(cx));
    
    // Perform UI operations
    terminal_view.update(cx, |view, cx| {
        view.execute_command("ls -la", cx);
    });
    
    // Assert visual correctness
    visual_test!("terminal_ls_command", &terminal_view, cx)?;
}
```

## Test Organization

### Directory Structure

```
project/
├── crates/
│   ├── terminal/
│   │   ├── src/
│   │   │   └── *.rs (with #[cfg(test)] modules)
│   │   ├── tests/
│   │   │   └── integration_tests.rs
│   │   └── benches/
│   │       └── terminal_benchmark.rs
│   └── workspace/
│       ├── src/
│       │   ├── hub_terminal_panel_tests.rs
│       │   └── *.rs
│       └── tests/
│           └── property_tests.rs
├── tests/
│   ├── integration/
│   │   └── terminal_workflows.rs
│   └── visual/
│       ├── baselines/
│       ├── output/
│       └── diffs/
└── docs/
    └── testing/
        ├── README.md
        ├── visual-testing.md
        ├── property-testing.md
        └── performance-testing.md
```

### Test Categories

#### 1. Unit Tests
- **Location**: `#[cfg(test)]` modules within source files
- **Purpose**: Test individual functions and methods
- **Example**: Terminal state management, command parsing

#### 2. Integration Tests
- **Location**: `tests/` directories within crates
- **Purpose**: Test component interactions
- **Example**: Terminal panel with state management

#### 3. Property-Based Tests
- **Location**: `tests/property_tests.rs`
- **Purpose**: Validate invariants across input ranges
- **Example**: Terminal state consistency, JSON serialization

#### 4. Performance Tests
- **Location**: `benches/` directories
- **Purpose**: Monitor performance and detect regressions
- **Example**: Terminal rendering, input processing

#### 5. Visual Tests
- **Location**: `tests/visual/`
- **Purpose**: Detect UI regressions
- **Example**: Terminal rendering, component layouts

## Running Tests

### All Tests
```bash
cargo nextest run --workspace
```

### Specific Test Types
```bash
# Unit and integration tests
cargo test --workspace

# Property-based tests (with more cases)
PROPTEST_CASES=10000 cargo test property_tests

# Performance benchmarks
cargo bench

# Visual tests (update baselines)
UPDATE_BASELINES=1 cargo test visual_tests
```

### CI/CD Integration

Tests are automatically run in GitHub Actions:

- **Pull Requests**: Full test suite on macOS and Linux
- **Main Branch**: Full test suite + performance regression detection
- **Nightly**: Extended test suite + fuzzing

## Writing Tests

### Best Practices

1. **Test Naming**: Use descriptive names that explain what is being tested
2. **Test Organization**: Group related tests in modules
3. **Test Data**: Use realistic test data that represents actual usage
4. **Assertions**: Make assertions specific and meaningful
5. **Test Isolation**: Ensure tests don't depend on each other

### Unit Test Example

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_state_creation() {
        let state = TerminalState::new(1);
        
        assert_eq!(state.terminal_id, 1);
        assert!(matches!(state.status, TerminalStatus::Initializing));
        assert!(state.commands.is_empty());
        assert!(state.working_directory.is_none());
    }

    #[test]
    fn test_command_execution_lifecycle() {
        let mut state = TerminalState::new(1);
        
        // Add command
        let command_id = state.add_command("echo hello".to_string());
        assert_eq!(state.commands.len(), 1);
        assert!(matches!(state.status, TerminalStatus::Busy));
        
        // Complete command
        state.complete_command(&command_id, 0);
        assert!(matches!(state.commands[0].status, CommandStatus::Success));
        assert!(matches!(state.status, TerminalStatus::Ready));
    }
}
```

### GPUI Test Example

```rust
#[gpui::test]
async fn test_hub_terminal_panel_initialization(cx: &mut TestAppContext) {
    let panel = cx.new_view(|cx| HubTerminalPanel::new(cx));

    panel.update(cx, |panel, _cx| {
        assert_eq!(panel.terminals.len(), 0);
        assert_eq!(panel.active_terminal_id, None);
        assert!(!panel.is_focused);
    });
}

#[gpui::test]
async fn test_terminal_command_execution(cx: &mut TestAppContext) {
    let panel = cx.new_view(|cx| HubTerminalPanel::new(cx));

    panel.update(cx, |panel, cx| {
        let terminal_id = panel.create_new_terminal(cx).unwrap();
        let command_id = panel.execute_command(
            terminal_id, 
            "echo test".to_string(), 
            cx
        ).unwrap();
        
        let state = panel.get_terminal_state(terminal_id).unwrap();
        assert_eq!(state.commands.len(), 1);
        assert_eq!(state.commands[0].command, "echo test");
    });
}
```

## Test Configuration

### Environment Variables

- `PROPTEST_CASES`: Number of property test cases (default: 256)
- `UPDATE_BASELINES`: Update visual test baselines when set
- `RUST_LOG`: Control logging level during tests
- `NEXTEST_PROFILE`: Use specific nextest profile

### Test Profiles

#### Development
```toml
[profile.dev]
test-threads = 1
retries = 0
```

#### CI
```toml
[profile.ci]
test-threads = 4
retries = 2
slow-timeout = "60s"
```

## Debugging Tests

### Failed Tests
```bash
# Run specific test with output
cargo test test_name -- --nocapture

# Run with debug logging
RUST_LOG=debug cargo test test_name

# Run single-threaded for debugging
cargo test test_name -- --test-threads=1
```

### Visual Test Failures
```bash
# Update baselines after reviewing diffs
UPDATE_BASELINES=1 cargo test visual_tests

# View diff images in tests/visual/diffs/
```

### Performance Test Analysis
```bash
# Generate detailed benchmark reports
cargo bench -- --output-format html

# Compare with baseline
cargo bench -- --save-baseline main
cargo bench -- --baseline main
```

## Continuous Integration

### GitHub Actions Workflow

The CI pipeline runs:

1. **Code Quality**: Clippy, formatting, license checks
2. **Unit Tests**: All unit and integration tests
3. **Property Tests**: Extended property-based testing
4. **Performance Tests**: Benchmark regression detection
5. **Visual Tests**: UI regression detection (on supported platforms)

### Test Reporting

- Test results are reported in GitHub PR checks
- Performance regression alerts are posted as comments
- Visual test failures include diff images as artifacts

## Maintenance

### Regular Tasks

1. **Update Baselines**: Review and update visual test baselines monthly
2. **Performance Monitoring**: Review benchmark trends weekly
3. **Test Coverage**: Monitor coverage and add tests for new features
4. **Dependency Updates**: Update test dependencies quarterly

### Test Health Metrics

- **Coverage**: Aim for >80% line coverage on critical paths
- **Performance**: No regressions >5% without justification
- **Flakiness**: <1% flaky test rate
- **Execution Time**: Full test suite <10 minutes

---

For specific testing guides, see:
- [Visual Testing Guide](visual-testing.md)
- [Property Testing Guide](property-testing.md)
- [Performance Testing Guide](performance-testing.md)

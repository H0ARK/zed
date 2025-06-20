//! Property-based tests for terminal state management
//! 
//! These tests use proptest to systematically validate terminal state invariants,
//! edge cases, and consistency properties across different scenarios.

use proptest::prelude::*;
use workspace::terminal_state::*;
use std::collections::HashMap;

// Property-based test strategies
prop_compose! {
    fn arb_command_string()(s in "[a-zA-Z0-9 ._-]{1,50}") -> String {
        s
    }
}

prop_compose! {
    fn arb_terminal_id()(id in 1u32..1000u32) -> u32 {
        id
    }
}

prop_compose! {
    fn arb_exit_code()(code in -128i32..128i32) -> i32 {
        code
    }
}

prop_compose! {
    fn arb_output_lines()(lines in prop::collection::vec("[a-zA-Z0-9 ._-]{0,100}", 0..20)) -> Vec<String> {
        lines
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Test that terminal state creation always produces valid initial state
    #[test]
    fn test_terminal_state_creation_invariants(terminal_id in arb_terminal_id()) {
        let state = TerminalState::new(terminal_id);
        
        // Invariants that must always hold for new terminal states
        prop_assert_eq!(state.terminal_id, terminal_id);
        prop_assert!(matches!(state.status, TerminalStatus::Initializing));
        prop_assert!(state.commands.is_empty());
        prop_assert!(state.working_directory.is_none());
        prop_assert!(!state.last_updated.is_empty());
    }

    /// Test that adding commands maintains state consistency
    #[test]
    fn test_add_command_invariants(
        terminal_id in arb_terminal_id(),
        commands in prop::collection::vec(arb_command_string(), 1..10)
    ) {
        let mut state = TerminalState::new(terminal_id);
        let mut command_ids = Vec::new();
        
        for (i, command) in commands.iter().enumerate() {
            let command_id = state.add_command(command.clone());
            command_ids.push(command_id.clone());
            
            // Invariants after adding each command
            prop_assert_eq!(state.commands.len(), i + 1);
            prop_assert!(matches!(state.status, TerminalStatus::Busy));
            
            let added_command = &state.commands[i];
            prop_assert_eq!(added_command.command, *command);
            prop_assert_eq!(added_command.id, command_id);
            prop_assert!(matches!(added_command.status, CommandStatus::Running));
            prop_assert!(added_command.output.is_empty());
            prop_assert_eq!(added_command.exit_code, None);
            prop_assert!(added_command.completed_at.is_none());
        }
        
        // All command IDs should be unique
        let mut unique_ids = std::collections::HashSet::new();
        for id in &command_ids {
            prop_assert!(unique_ids.insert(id.clone()), "Duplicate command ID: {}", id);
        }
    }

    /// Test that command completion maintains consistency
    #[test]
    fn test_command_completion_invariants(
        terminal_id in arb_terminal_id(),
        command in arb_command_string(),
        exit_code in arb_exit_code(),
        output in arb_output_lines()
    ) {
        let mut state = TerminalState::new(terminal_id);
        let command_id = state.add_command(command.clone());
        
        // Update command output before completion
        state.update_command_output(&command_id, output.clone());
        
        // Complete the command
        state.complete_command(&command_id, exit_code);
        
        // Invariants after completion
        prop_assert_eq!(state.commands.len(), 1);
        
        let completed_command = &state.commands[0];
        prop_assert_eq!(completed_command.command, command);
        prop_assert_eq!(completed_command.id, command_id);
        prop_assert_eq!(completed_command.exit_code, Some(exit_code));
        prop_assert!(completed_command.completed_at.is_some());
        prop_assert_eq!(completed_command.output, output);
        
        // Status should match exit code
        if exit_code == 0 {
            prop_assert!(matches!(completed_command.status, CommandStatus::Success));
        } else {
            prop_assert!(matches!(completed_command.status, CommandStatus::Error));
        }
        
        // Terminal status should be ready when no commands are running
        prop_assert!(matches!(state.status, TerminalStatus::Ready));
    }

    /// Test JSON serialization round-trip consistency
    #[test]
    fn test_json_serialization_roundtrip(
        terminal_id in arb_terminal_id(),
        commands in prop::collection::vec(
            (arb_command_string(), arb_exit_code(), arb_output_lines()), 
            0..5
        )
    ) {
        let mut state = TerminalState::new(terminal_id);
        
        // Add and complete commands
        for (command, exit_code, output) in commands {
            let command_id = state.add_command(command);
            state.update_command_output(&command_id, output);
            state.complete_command(&command_id, exit_code);
        }
        
        // Serialize to JSON
        let json = state.to_json().unwrap();
        
        // Deserialize back
        let deserialized = TerminalState::from_json(&json).unwrap();
        
        // Verify round-trip consistency
        prop_assert_eq!(state.terminal_id, deserialized.terminal_id);
        prop_assert_eq!(state.commands.len(), deserialized.commands.len());
        prop_assert_eq!(state.working_directory, deserialized.working_directory);
        
        // Verify each command is preserved
        for (original, deserialized) in state.commands.iter().zip(deserialized.commands.iter()) {
            prop_assert_eq!(original.id, deserialized.id);
            prop_assert_eq!(original.command, deserialized.command);
            prop_assert_eq!(original.exit_code, deserialized.exit_code);
            prop_assert_eq!(original.output, deserialized.output);
            
            // Status should be preserved (using debug comparison for enum)
            prop_assert_eq!(format!("{:?}", original.status), format!("{:?}", deserialized.status));
        }
    }

    /// Test that command output updates are consistent
    #[test]
    fn test_command_output_update_invariants(
        terminal_id in arb_terminal_id(),
        command in arb_command_string(),
        output_batches in prop::collection::vec(arb_output_lines(), 1..5)
    ) {
        let mut state = TerminalState::new(terminal_id);
        let command_id = state.add_command(command);
        
        let mut expected_output = Vec::new();
        
        for batch in output_batches {
            expected_output.extend(batch.clone());
            state.update_command_output(&command_id, batch);
            
            // Verify output accumulates correctly
            prop_assert_eq!(state.commands[0].output, expected_output);
            prop_assert!(matches!(state.commands[0].status, CommandStatus::Running));
        }
    }

    /// Test terminal state with multiple concurrent commands
    #[test]
    fn test_multiple_commands_invariants(
        terminal_id in arb_terminal_id(),
        commands in prop::collection::vec(arb_command_string(), 2..10)
    ) {
        let mut state = TerminalState::new(terminal_id);
        let mut command_ids = Vec::new();
        
        // Add multiple commands
        for command in &commands {
            let command_id = state.add_command(command.clone());
            command_ids.push(command_id);
        }
        
        // All commands should be running initially
        prop_assert_eq!(state.commands.len(), commands.len());
        prop_assert!(matches!(state.status, TerminalStatus::Busy));
        
        for cmd in &state.commands {
            prop_assert!(matches!(cmd.status, CommandStatus::Running));
        }
        
        // Complete commands one by one
        for (i, command_id) in command_ids.iter().enumerate() {
            state.complete_command(command_id, 0);
            
            // Check that the specific command is completed
            let completed_cmd = state.commands.iter().find(|c| &c.id == command_id).unwrap();
            prop_assert!(matches!(completed_cmd.status, CommandStatus::Success));
            prop_assert_eq!(completed_cmd.exit_code, Some(0));
        }
        
        // After all commands complete, terminal should be ready
        prop_assert!(matches!(state.status, TerminalStatus::Ready));
        
        // All commands should be successful
        for cmd in &state.commands {
            prop_assert!(matches!(cmd.status, CommandStatus::Success));
        }
    }

    /// Test edge cases with empty and special characters
    #[test]
    fn test_edge_case_commands(
        terminal_id in arb_terminal_id(),
        special_commands in prop::collection::vec(
            prop::option::of("[\\x00-\\x1F\\x7F-\\xFF]{0,10}"), 
            0..3
        )
    ) {
        let mut state = TerminalState::new(terminal_id);
        
        for maybe_command in special_commands {
            if let Some(command) = maybe_command {
                // Even with special characters, basic invariants should hold
                let command_id = state.add_command(command.clone());
                
                prop_assert!(!command_id.is_empty());
                prop_assert!(state.commands.len() > 0);
                
                let added_cmd = state.commands.last().unwrap();
                prop_assert_eq!(added_cmd.command, command);
                prop_assert!(matches!(added_cmd.status, CommandStatus::Running));
            }
        }
    }

    /// Test timestamp consistency
    #[test]
    fn test_timestamp_consistency(
        terminal_id in arb_terminal_id(),
        command in arb_command_string()
    ) {
        let mut state = TerminalState::new(terminal_id);
        let initial_timestamp = state.last_updated.clone();
        
        // Adding a command should update timestamp
        let command_id = state.add_command(command);
        prop_assert_ne!(state.last_updated, initial_timestamp);
        
        let after_add_timestamp = state.last_updated.clone();
        
        // Updating output should update timestamp
        state.update_command_output(&command_id, vec!["output".to_string()]);
        prop_assert_ne!(state.last_updated, after_add_timestamp);
        
        let after_output_timestamp = state.last_updated.clone();
        
        // Completing command should update timestamp
        state.complete_command(&command_id, 0);
        prop_assert_ne!(state.last_updated, after_output_timestamp);
        
        // All timestamps should be valid (parseable as numbers)
        prop_assert!(initial_timestamp.parse::<u64>().is_ok());
        prop_assert!(after_add_timestamp.parse::<u64>().is_ok());
        prop_assert!(after_output_timestamp.parse::<u64>().is_ok());
        prop_assert!(state.last_updated.parse::<u64>().is_ok());
    }
}

#[cfg(test)]
mod integration_property_tests {
    use super::*;
    use std::collections::HashMap;

    /// Test that multiple terminal states don't interfere with each other
    #[test]
    fn test_multiple_terminal_states_isolation() {
        proptest!(|(
            terminal_ids in prop::collection::vec(arb_terminal_id(), 2..5),
            commands_per_terminal in prop::collection::vec(
                prop::collection::vec(arb_command_string(), 1..3), 
                2..5
            )
        )| {
            let mut states: HashMap<u32, TerminalState> = HashMap::new();
            
            // Create multiple terminal states
            for (&terminal_id, commands) in terminal_ids.iter().zip(commands_per_terminal.iter()) {
                let mut state = TerminalState::new(terminal_id);
                
                for command in commands {
                    let command_id = state.add_command(command.clone());
                    state.complete_command(&command_id, 0);
                }
                
                states.insert(terminal_id, state);
            }
            
            // Verify isolation - each terminal state should be independent
            for (&terminal_id, state) in &states {
                prop_assert_eq!(state.terminal_id, terminal_id);
                
                // Commands should only belong to this terminal
                for cmd in &state.commands {
                    prop_assert!(cmd.id.contains(&terminal_id.to_string()));
                }
                
                // State should not be affected by other terminals
                prop_assert!(matches!(state.status, TerminalStatus::Ready));
            }
            
            // All terminal IDs should be unique
            let unique_ids: std::collections::HashSet<_> = states.keys().collect();
            prop_assert_eq!(unique_ids.len(), states.len());
        });
    }
}

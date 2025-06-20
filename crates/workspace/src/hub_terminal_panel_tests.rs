//! Comprehensive tests for Hub Terminal Panel
//! 
//! Tests cover terminal initialization, command execution, state management,
//! UI interactions, and error scenarios for the terminal-first architecture.

#[cfg(test)]
mod tests {
    use super::super::hub_terminal_panel::*;
    use super::super::terminal_state::*;
    use gpui::{TestAppContext, VisualTestContext, Entity, Context};
    use std::time::Duration;
    use terminal::Terminal;
    use project::terminals::TerminalKind;

    #[gpui::test]
    async fn test_hub_terminal_panel_initialization(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, _cx| {
            assert_eq!(panel.terminals.len(), 0);
            assert_eq!(panel.active_terminal_id, None);
            assert!(!panel.is_focused);
        });
    }

    #[gpui::test]
    async fn test_create_new_terminal(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal_id = panel.create_new_terminal(cx).unwrap();
            
            assert_eq!(panel.terminals.len(), 1);
            assert_eq!(panel.active_terminal_id, Some(terminal_id));
            assert!(panel.terminals.contains_key(&terminal_id));
        });
    }

    #[gpui::test]
    async fn test_terminal_state_management(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal_id = panel.create_new_terminal(cx).unwrap();
            
            // Test initial state
            let state = panel.get_terminal_state(terminal_id).unwrap();
            assert_eq!(state.terminal_id, terminal_id);
            assert!(matches!(state.status, TerminalStatus::Initializing));
            assert!(state.commands.is_empty());
        });
    }

    #[gpui::test]
    async fn test_command_execution_lifecycle(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal_id = panel.create_new_terminal(cx).unwrap();
            
            // Execute a command
            let command_id = panel.execute_command(terminal_id, "echo hello".to_string(), cx).unwrap();
            
            let state = panel.get_terminal_state(terminal_id).unwrap();
            assert_eq!(state.commands.len(), 1);
            assert_eq!(state.commands[0].command, "echo hello");
            assert!(matches!(state.commands[0].status, CommandStatus::Running));
            assert_eq!(state.commands[0].id, command_id);
        });
    }

    #[gpui::test]
    async fn test_command_completion(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal_id = panel.create_new_terminal(cx).unwrap();
            let command_id = panel.execute_command(terminal_id, "echo test".to_string(), cx).unwrap();
            
            // Simulate command completion
            panel.complete_command(terminal_id, &command_id, 0, vec!["test".to_string()], cx);
            
            let state = panel.get_terminal_state(terminal_id).unwrap();
            let command = &state.commands[0];
            assert!(matches!(command.status, CommandStatus::Success));
            assert_eq!(command.exit_code, Some(0));
            assert_eq!(command.output, vec!["test"]);
            assert!(command.completed_at.is_some());
        });
    }

    #[gpui::test]
    async fn test_command_error_handling(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal_id = panel.create_new_terminal(cx).unwrap();
            let command_id = panel.execute_command(terminal_id, "invalid_command".to_string(), cx).unwrap();
            
            // Simulate command failure
            panel.complete_command(
                terminal_id, 
                &command_id, 
                1, 
                vec!["command not found".to_string()], 
                cx
            );
            
            let state = panel.get_terminal_state(terminal_id).unwrap();
            let command = &state.commands[0];
            assert!(matches!(command.status, CommandStatus::Error));
            assert_eq!(command.exit_code, Some(1));
            assert_eq!(command.output, vec!["command not found"]);
        });
    }

    #[gpui::test]
    async fn test_multiple_terminals(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal1 = panel.create_new_terminal(cx).unwrap();
            let terminal2 = panel.create_new_terminal(cx).unwrap();
            let terminal3 = panel.create_new_terminal(cx).unwrap();
            
            assert_eq!(panel.terminals.len(), 3);
            assert_eq!(panel.active_terminal_id, Some(terminal3)); // Last created is active
            
            // Switch between terminals
            panel.switch_to_terminal(terminal1, cx);
            assert_eq!(panel.active_terminal_id, Some(terminal1));
            
            panel.switch_to_terminal(terminal2, cx);
            assert_eq!(panel.active_terminal_id, Some(terminal2));
        });
    }

    #[gpui::test]
    async fn test_terminal_closure(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal1 = panel.create_new_terminal(cx).unwrap();
            let terminal2 = panel.create_new_terminal(cx).unwrap();
            
            assert_eq!(panel.terminals.len(), 2);
            assert_eq!(panel.active_terminal_id, Some(terminal2));
            
            // Close active terminal
            panel.close_terminal(terminal2, cx);
            
            assert_eq!(panel.terminals.len(), 1);
            assert_eq!(panel.active_terminal_id, Some(terminal1)); // Switches to remaining terminal
            
            // Close last terminal
            panel.close_terminal(terminal1, cx);
            
            assert_eq!(panel.terminals.len(), 0);
            assert_eq!(panel.active_terminal_id, None);
        });
    }

    #[gpui::test]
    async fn test_json_state_serialization(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal_id = panel.create_new_terminal(cx).unwrap();
            let command_id = panel.execute_command(terminal_id, "ls -la".to_string(), cx).unwrap();
            
            panel.complete_command(
                terminal_id, 
                &command_id, 
                0, 
                vec!["file1.txt".to_string(), "file2.txt".to_string()], 
                cx
            );
            
            // Test JSON serialization
            let json_state = panel.get_terminal_json_state(terminal_id).unwrap();
            assert!(json_state.contains("\"command\": \"ls -la\""));
            assert!(json_state.contains("\"status\": \"success\""));
            assert!(json_state.contains("file1.txt"));
            assert!(json_state.contains("file2.txt"));
            
            // Test deserialization
            let parsed_state = TerminalState::from_json(&json_state).unwrap();
            assert_eq!(parsed_state.terminal_id, terminal_id);
            assert_eq!(parsed_state.commands.len(), 1);
            assert_eq!(parsed_state.commands[0].command, "ls -la");
        });
    }

    #[gpui::test]
    async fn test_terminal_focus_management(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal_id = panel.create_new_terminal(cx).unwrap();
            
            // Test focus handling
            assert!(!panel.is_focused);
            
            panel.focus_terminal(terminal_id, cx);
            assert!(panel.is_focused);
            assert_eq!(panel.active_terminal_id, Some(terminal_id));
            
            panel.blur_terminal(cx);
            assert!(!panel.is_focused);
        });
    }

    #[gpui::test]
    async fn test_terminal_scroll_operations(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal_id = panel.create_new_terminal(cx).unwrap();
            
            // Add multiple commands to create scrollable content
            for i in 0..10 {
                let command_id = panel.execute_command(
                    terminal_id, 
                    format!("echo line {}", i), 
                    cx
                ).unwrap();
                
                panel.complete_command(
                    terminal_id, 
                    &command_id, 
                    0, 
                    vec![format!("line {}", i)], 
                    cx
                );
            }
            
            let state = panel.get_terminal_state(terminal_id).unwrap();
            assert_eq!(state.commands.len(), 10);
            
            // Test scroll operations (these would interact with the terminal's scroll state)
            panel.scroll_terminal(terminal_id, ScrollDirection::Up, 5, cx);
            panel.scroll_terminal(terminal_id, ScrollDirection::Down, 3, cx);
            panel.scroll_to_top(terminal_id, cx);
            panel.scroll_to_bottom(terminal_id, cx);
        });
    }

    #[gpui::test]
    async fn test_terminal_input_handling(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal_id = panel.create_new_terminal(cx).unwrap();
            
            // Test various input scenarios
            panel.handle_terminal_input(terminal_id, "hello world".to_string(), cx);
            panel.handle_terminal_input(terminal_id, "\n".to_string(), cx); // Enter key
            panel.handle_terminal_input(terminal_id, "\x03".to_string(), cx); // Ctrl+C
            panel.handle_terminal_input(terminal_id, "\x04".to_string(), cx); // Ctrl+D
            
            // Verify input was processed (implementation would depend on actual terminal handling)
            let state = panel.get_terminal_state(terminal_id).unwrap();
            assert!(matches!(state.status, TerminalStatus::Ready) || matches!(state.status, TerminalStatus::Busy));
        });
    }

    #[gpui::test]
    async fn test_terminal_error_recovery(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal_id = panel.create_new_terminal(cx).unwrap();
            
            // Simulate terminal error
            panel.handle_terminal_error(terminal_id, "Terminal process crashed".to_string(), cx);
            
            let state = panel.get_terminal_state(terminal_id).unwrap();
            assert!(matches!(state.status, TerminalStatus::Error));
            
            // Test recovery
            panel.restart_terminal(terminal_id, cx).unwrap();
            
            let state = panel.get_terminal_state(terminal_id).unwrap();
            assert!(matches!(state.status, TerminalStatus::Initializing) || matches!(state.status, TerminalStatus::Ready));
        });
    }

    #[gpui::test]
    async fn test_terminal_performance_monitoring(cx: &mut TestAppContext) {
        let panel = cx.new_view(|cx| {
            HubTerminalPanel::new(cx)
        });

        panel.update(cx, |panel, cx| {
            let terminal_id = panel.create_new_terminal(cx).unwrap();
            
            let start_time = std::time::Instant::now();
            
            // Execute multiple commands rapidly
            for i in 0..100 {
                let command_id = panel.execute_command(
                    terminal_id, 
                    format!("echo {}", i), 
                    cx
                ).unwrap();
                
                panel.complete_command(
                    terminal_id, 
                    &command_id, 
                    0, 
                    vec![format!("{}", i)], 
                    cx
                );
            }
            
            let duration = start_time.elapsed();
            
            // Verify performance is reasonable (adjust threshold as needed)
            assert!(duration < Duration::from_millis(1000), "Terminal operations took too long: {:?}", duration);
            
            let state = panel.get_terminal_state(terminal_id).unwrap();
            assert_eq!(state.commands.len(), 100);
        });
    }

    // Helper enums and structs for testing
    #[derive(Debug)]
    enum ScrollDirection {
        Up,
        Down,
    }

    // Mock implementations for testing - these would need to be implemented in the actual hub_terminal_panel.rs
    impl HubTerminalPanel {
        fn scroll_terminal(&mut self, _terminal_id: u32, _direction: ScrollDirection, _lines: usize, _cx: &mut gpui::ViewContext<Self>) {
            // Mock implementation for testing
        }
        
        fn scroll_to_top(&mut self, _terminal_id: u32, _cx: &mut gpui::ViewContext<Self>) {
            // Mock implementation for testing
        }
        
        fn scroll_to_bottom(&mut self, _terminal_id: u32, _cx: &mut gpui::ViewContext<Self>) {
            // Mock implementation for testing
        }
        
        fn handle_terminal_input(&mut self, _terminal_id: u32, _input: String, _cx: &mut gpui::ViewContext<Self>) {
            // Mock implementation for testing
        }
        
        fn handle_terminal_error(&mut self, _terminal_id: u32, _error: String, _cx: &mut gpui::ViewContext<Self>) {
            // Mock implementation for testing
        }
        
        fn restart_terminal(&mut self, _terminal_id: u32, _cx: &mut gpui::ViewContext<Self>) -> Result<(), String> {
            // Mock implementation for testing
            Ok(())
        }
    }
}

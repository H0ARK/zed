# Protocol Specification

**The Hub CLI ↔ UI Communication Protocol**

## Overview

The Hub Protocol is a JSON-based communication standard that enables CLI applications to send rich UI components and interactions to The Hub terminal interface. The protocol is designed to be:

- **Backward compatible**: CLI tools work normally when protocol is unavailable
- **Progressive**: Applications can adopt features incrementally
- **Extensible**: New UI components can be added without breaking existing tools
- **Efficient**: Minimal overhead and fast rendering
- **Secure**: Sandboxed execution with permission controls

## Protocol Architecture

### Connection Model

CLI applications connect to The Hub through one of several transport mechanisms:

1. **Environment Variable Detection**
   - `HUB_SOCKET` - Unix domain socket path
   - `HUB_PORT` - TCP port for remote connections
   - `HUB_SESSION` - Session identifier for multiplexing

2. **Auto-Discovery**
   - Standard socket locations (`~/.the-hub/socket`)
   - Process tree inspection for The Hub parent processes
   - Network discovery for remote sessions

3. **Explicit Connection**
   - CLI applications can explicitly connect via SDK
   - Support for both synchronous and asynchronous communication

### Message Format

All protocol messages use JSON with a standardized envelope:

```json
{
  "version": "1.0",
  "type": "ui_message",
  "session_id": "uuid-string",
  "sequence": 123,
  "timestamp": 1703875200000,
  "payload": {
    // Message-specific content
  }
}
```

#### Envelope Fields

- **version**: Protocol version for compatibility handling
- **type**: Message category (`ui_message`, `control`, `response`, `event`)
- **session_id**: Unique identifier for the command session
- **sequence**: Incrementing number for message ordering
- **timestamp**: Unix timestamp for temporal ordering
- **payload**: Message-specific data structure

## Core Message Types

### 1. Session Management

#### Session Start
```json
{
  "type": "control",
  "payload": {
    "action": "session_start",
    "command": "git status",
    "args": ["--porcelain"],
    "cwd": "/path/to/repo",
    "capabilities": {
      "ui_components": ["table", "progress", "form"],
      "interactions": ["click", "select", "input"],
      "ai_integration": true
    }
  }
}
```

#### Session End
```json
{
  "type": "control",
  "payload": {
    "action": "session_end",
    "exit_code": 0,
    "duration_ms": 1250,
    "summary": "Status check completed successfully"
  }
}
```

### 2. UI Components

#### Progress Indicator
```json
{
  "type": "ui_message",
  "payload": {
    "component": "progress",
    "props": {
      "current": 75,
      "total": 100,
      "message": "Downloading dependencies...",
      "show_percentage": true,
      "show_eta": true,
      "style": "bar" // "bar", "spinner", "dots"
    }
  }
}
```

#### Data Table
```json
{
  "type": "ui_message",
  "payload": {
    "component": "table",
    "props": {
      "headers": [
        {"text": "File", "width": "flex"},
        {"text": "Status", "width": "100px"},
        {"text": "Changes", "width": "80px"}
      ],
      "rows": [
        {
          "id": "file-1",
          "cells": ["src/main.rs", "Modified", "+15 -3"],
          "actions": ["view_diff", "stage", "discard"],
          "status": "warning"
        }
      ],
      "sortable": true,
      "filterable": true,
      "selectable": "multiple"
    }
  }
}
```

#### File Tree
```json
{
  "type": "ui_message",
  "payload": {
    "component": "file_tree",
    "props": {
      "root": "/path/to/project",
      "entries": [
        {
          "path": "src/",
          "type": "directory",
          "expanded": true,
          "children": [
            {
              "path": "src/main.rs",
              "type": "file",
              "size": 2048,
              "modified": "2023-12-01T10:30:00Z",
              "status": "modified",
              "actions": ["open", "diff"]
            }
          ]
        }
      ],
      "show_hidden": false,
      "icons": true
    }
  }
}
```

#### Interactive Form
```json
{
  "type": "ui_message",
  "payload": {
    "component": "form",
    "props": {
      "title": "Commit Changes",
      "fields": [
        {
          "name": "message",
          "type": "textarea",
          "label": "Commit Message",
          "required": true,
          "placeholder": "Enter commit message..."
        },
        {
          "name": "files",
          "type": "checkbox_group",
          "label": "Files to Include",
          "options": [
            {"value": "src/main.rs", "label": "src/main.rs", "checked": true},
            {"value": "README.md", "label": "README.md", "checked": false}
          ]
        }
      ],
      "actions": [
        {"label": "Commit", "action": "commit", "style": "primary"},
        {"label": "Cancel", "action": "cancel", "style": "secondary"}
      ]
    }
  }
}
```

#### Status Cards
```json
{
  "type": "ui_message",
  "payload": {
    "component": "status_grid",
    "props": {
      "cards": [
        {
          "title": "Build Status",
          "status": "success",
          "primary_metric": "✓ Passed",
          "secondary_metrics": ["2.3s", "0 warnings"],
          "actions": ["view_logs", "rebuild"]
        },
        {
          "title": "Test Coverage",
          "status": "warning",
          "primary_metric": "78%",
          "secondary_metrics": ["↓ 2%", "15 uncovered"],
          "actions": ["view_report", "run_tests"]
        }
      ]
    }
  }
}
```

### 3. Real-time Updates

#### Component Update
```json
{
  "type": "ui_message",
  "payload": {
    "component": "update",
    "target": "progress-bar-1",
    "props": {
      "current": 85,
      "message": "Almost complete..."
    }
  }
}
```

#### Streaming Data
```json
{
  "type": "ui_message",
  "payload": {
    "component": "stream",
    "stream_id": "build-logs",
    "data": {
      "type": "log_line",
      "content": "Compiling main.rs...",
      "level": "info",
      "timestamp": "2023-12-01T10:30:15Z"
    }
  }
}
```

### 4. User Interactions

#### User Input Response
```json
{
  "type": "response",
  "payload": {
    "interaction_id": "form-commit-1",
    "action": "commit",
    "data": {
      "message": "Fix parsing bug in command processor",
      "files": ["src/main.rs", "tests/parser_test.rs"]
    }
  }
}
```

#### Selection Events
```json
{
  "type": "event",
  "payload": {
    "event": "selection_changed",
    "component_id": "file-table-1",
    "selected_ids": ["file-1", "file-3"],
    "selection_type": "multiple"
  }
}
```

## UI Component Library

### Layout Components

- **Container**: Flexible layout container with padding and spacing
- **Grid**: Responsive grid system for organized layouts
- **Tabs**: Tabbed interface for organizing related content
- **Accordion**: Collapsible content sections
- **Split Panel**: Resizable split layouts

### Data Display

- **Table**: Sortable, filterable data tables with actions
- **List**: Simple and complex list displays
- **Tree**: Hierarchical data representation
- **Timeline**: Chronological event display
- **Metrics Dashboard**: Key performance indicators

### Input Components

- **Form**: Complete form handling with validation
- **Text Input**: Single and multi-line text inputs
- **Select**: Dropdown and multi-select components
- **Checkbox/Radio**: Selection controls
- **File Picker**: File and directory selection

### Feedback Components

- **Progress**: Progress bars, spinners, and step indicators
- **Notifications**: Toast messages and alerts
- **Status Indicators**: Success, warning, error states
- **Loading States**: Various loading representations

### Interactive Components

- **Buttons**: Action buttons with various styles
- **Menu**: Context menus and action menus
- **Modal**: Overlay dialogs and confirmations
- **Tooltip**: Contextual help and information

## Protocol Extensions

### AI Integration Messages

#### AI Suggestion
```json
{
  "type": "ai_message",
  "payload": {
    "suggestion_type": "command_optimization",
    "context": "git status command slow on large repo",
    "suggestion": "Use 'git status --porcelain' for faster machine-readable output",
    "confidence": 0.85,
    "actions": ["apply_suggestion", "learn_more", "dismiss"]
  }
}
```

#### Context Sharing
```json
{
  "type": "ai_message",
  "payload": {
    "context_type": "command_history",
    "data": {
      "recent_commands": ["git add .", "git status", "npm test"],
      "current_directory": "/path/to/project",
      "project_type": "javascript",
      "repository_info": {
        "branch": "feature/new-parser",
        "has_changes": true
      }
    }
  }
}
```

### Collaboration Features

#### Session Sharing
```json
{
  "type": "collaboration",
  "payload": {
    "action": "share_session",
    "share_code": "ABC123",
    "permissions": ["view", "interact"],
    "expires_at": "2023-12-01T12:00:00Z"
  }
}
```

#### Live Cursors
```json
{
  "type": "collaboration",
  "payload": {
    "action": "cursor_update",
    "user_id": "user-456",
    "position": {
      "component_id": "file-table-1",
      "row": 3,
      "column": 1
    }
  }
}
```

## Error Handling

### Protocol Errors
```json
{
  "type": "error",
  "payload": {
    "error_code": "INVALID_COMPONENT",
    "message": "Unknown component type 'custom_widget'",
    "details": {
      "supported_components": ["table", "progress", "form", "..."],
      "suggestion": "Use 'table' component for tabular data"
    }
  }
}
```

### Graceful Degradation
When The Hub is unavailable or doesn't support specific features:

1. **Fallback Mode**: CLI applications detect absence and use traditional text output
2. **Feature Detection**: Query supported capabilities before using advanced features
3. **Progressive Enhancement**: Start with basic UI and add richness when available

## Security Considerations

### Sandboxing
- UI components run in isolated rendering contexts
- No direct access to system resources
- Validated input sanitization

### Permissions
- Granular permissions for different UI capabilities
- User approval for sensitive operations
- Audit logging for security-relevant actions

### Data Privacy
- Local-first operation by default
- Encrypted transmission for remote sessions
- Configurable data retention policies

## Performance Optimization

### Message Batching
```json
{
  "type": "batch",
  "payload": {
    "messages": [
      // Multiple UI messages in single transmission
    ]
  }
}
```

### Efficient Updates
- Incremental UI updates rather than full re-renders
- Virtual scrolling for large data sets
- Lazy loading for complex components

### Resource Management
- Memory limits for UI components
- CPU throttling for expensive operations
- Network optimization for remote sessions

## Versioning and Compatibility

### Protocol Versioning
- Semantic versioning for protocol changes
- Backward compatibility guarantees
- Feature negotiation during connection

### Component Evolution
- Additive changes preserve compatibility
- Deprecation warnings for obsolete features
- Migration tools for major version updates

## Implementation Guidelines

### SDK Integration
- Minimal overhead when protocol unavailable
- Async/non-blocking by default
- Comprehensive error handling

### Testing Strategy
- Protocol compliance testing
- UI component validation
- Performance benchmarking

### Documentation Standards
- Complete API reference
- Interactive examples
- Migration guides
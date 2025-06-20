# UI Component Library

**Comprehensive reference for Zed-Hub's rich UI components and interaction patterns**

## Component Design Philosophy

### 1. Command-Line Native
Every component is designed specifically for command-line workflows:
- **Fast keyboard navigation** with intuitive shortcuts
- **Information density** optimized for developer needs
- **Monospace-friendly** layouts and typography
- **Terminal-appropriate** color schemes and contrast

### 2. Progressive Disclosure
Components reveal complexity gradually:
- **Simple by default** with advanced options available
- **Contextual actions** appear when relevant
- **Expandable sections** for detailed information
- **Layered interactions** from basic to advanced

### 3. Consistency with Power
Standardized patterns with customization options:
- **Uniform interaction models** across all components
- **Consistent visual language** with thematic flexibility
- **Predictable behavior** that builds user confidence
- **Extensible architecture** for custom needs

## Core Component Categories

### Data Display Components

#### Table Component
The most versatile component for structured data display with rich interaction capabilities.

**Basic Table**
```json
{
  "component": "table",
  "props": {
    "headers": [
      {"text": "File", "width": "flex", "sortable": true},
      {"text": "Size", "width": "100px", "sortable": true, "align": "right"},
      {"text": "Modified", "width": "150px", "sortable": true},
      {"text": "Status", "width": "80px", "filterable": true}
    ],
    "rows": [
      {
        "id": "file-1",
        "cells": ["src/main.rs", "2.1 KB", "2 hours ago", "Modified"],
        "status": "warning",
        "actions": ["edit", "diff", "stage"]
      }
    ]
  }
}
```

**Advanced Table Features**
- **Sorting**: Multi-column sorting with visual indicators
- **Filtering**: Column-specific filters with type awareness
- **Selection**: Single, multiple, and range selection modes
- **Pagination**: Virtual scrolling for large datasets
- **Grouping**: Hierarchical data organization
- **Custom Renderers**: Rich cell content with embedded components

**Table Interaction Patterns**
- `↑/↓` - Navigate rows
- `←/→` - Navigate columns
- `Space` - Toggle row selection
- `Enter` - Activate primary action
- `Ctrl+A` - Select all visible rows
- `Ctrl+F` - Open filter dialog
- `Ctrl+S` - Sort by current column

#### List Component
Simplified data display for linear information.

**Simple List**
```json
{
  "component": "list",
  "props": {
    "items": [
      {
        "id": "item-1",
        "primary": "Feature: Add user authentication",
        "secondary": "branch: feature/auth • 2 commits ahead",
        "icon": "git-branch",
        "status": "active",
        "actions": ["checkout", "merge", "delete"]
      }
    ],
    "selectable": true,
    "searchable": true
  }
}
```

**List Variants**
- **Icon List**: Items with status or type icons
- **Detailed List**: Primary and secondary text with metadata
- **Action List**: Items with inline action buttons
- **Grouped List**: Sections with headers and collapsible groups

#### Tree Component
Hierarchical data display with expansion and navigation.

**File Tree**
```json
{
  "component": "tree",
  "props": {
    "root": {
      "id": "project-root",
      "label": "my-project/",
      "type": "directory",
      "expanded": true,
      "children": [
        {
          "id": "src-dir",
          "label": "src/",
          "type": "directory",
          "expanded": false,
          "children": [
            {
              "id": "main-rs",
              "label": "main.rs",
              "type": "file",
              "status": "modified",
              "actions": ["open", "diff"]
            }
          ]
        }
      ]
    },
    "icons": true,
    "checkbox_selection": false,
    "lazy_loading": true
  }
}
```

**Tree Features**
- **Lazy Loading**: Load children on expansion
- **Drag and Drop**: Reorder and move items
- **Multi-Selection**: Complex selection patterns
- **Custom Icons**: File type and status indicators
- **Search**: Find and highlight items

### Input Components

#### Form Component
Comprehensive form handling with validation and rich input types.

**Complete Form Example**
```json
{
  "component": "form",
  "props": {
    "title": "Deploy Application",
    "description": "Configure deployment settings for your application",
    "sections": [
      {
        "title": "Environment",
        "fields": [
          {
            "name": "environment",
            "type": "select",
            "label": "Target Environment",
            "required": true,
            "options": [
              {"value": "dev", "label": "Development"},
              {"value": "staging", "label": "Staging"},
              {"value": "prod", "label": "Production"}
            ],
            "default": "staging"
          },
          {
            "name": "region",
            "type": "select",
            "label": "AWS Region",
            "options": [
              {"value": "us-east-1", "label": "US East (N. Virginia)"},
              {"value": "us-west-2", "label": "US West (Oregon)"},
              {"value": "eu-west-1", "label": "Europe (Ireland)"}
            ],
            "default": "us-east-1",
            "help": "Choose the region closest to your users"
          }
        ]
      },
      {
        "title": "Configuration",
        "fields": [
          {
            "name": "instances",
            "type": "number",
            "label": "Instance Count",
            "min": 1,
            "max": 10,
            "default": 2,
            "step": 1
          },
          {
            "name": "auto_scale",
            "type": "checkbox",
            "label": "Enable Auto Scaling",
            "default": true
          },
          {
            "name": "health_check_path",
            "type": "text",
            "label": "Health Check Path",
            "placeholder": "/health",
            "pattern": "^/.*",
            "validation_message": "Path must start with /"
          }
        ]
      }
    ],
    "actions": [
      {
        "label": "Deploy",
        "action": "deploy",
        "style": "primary",
        "confirm": "Are you sure you want to deploy to production?"
      },
      {
        "label": "Save Draft",
        "action": "save_draft",
        "style": "secondary"
      },
      {
        "label": "Cancel",
        "action": "cancel",
        "style": "ghost"
      }
    ]
  }
}
```

**Input Field Types**
- **Text**: Single-line text with validation patterns
- **Textarea**: Multi-line text with character limits
- **Number**: Numeric input with min/max/step constraints
- **Select**: Dropdown with search and multi-select options
- **Checkbox**: Boolean toggle with optional label
- **Radio**: Single selection from multiple options
- **File**: File picker with type and size restrictions
- **Password**: Masked text input with strength indicators
- **Date/Time**: Date and time pickers with formatting
- **Slider**: Numeric range selection with visual feedback

#### Quick Input Component
Fast, lightweight input for simple interactions.

**Command Palette Style**
```json
{
  "component": "quick_input",
  "props": {
    "placeholder": "Type a command or search...",
    "suggestions": [
      {
        "text": "git status",
        "description": "Show working tree status",
        "category": "Git",
        "score": 100
      },
      {
        "text": "npm install",
        "description": "Install dependencies",
        "category": "NPM",
        "score": 95
      }
    ],
    "fuzzy_search": true,
    "categories": ["Git", "NPM", "Docker", "System"],
    "recent_items": true
  }
}
```

### Feedback Components

#### Progress Component
Visual feedback for long-running operations with rich status information.

**Advanced Progress Tracking**
```json
{
  "component": "progress",
  "props": {
    "type": "multi_step",
    "overall_progress": 60,
    "current_step": 2,
    "total_steps": 5,
    "steps": [
      {
        "title": "Preparing build",
        "status": "completed",
        "duration": "2.3s"
      },
      {
        "title": "Compiling sources", 
        "status": "completed",
        "duration": "45.7s",
        "details": "142 files compiled"
      },
      {
        "title": "Running tests",
        "status": "in_progress",
        "progress": 70,
        "details": "35/50 tests completed",
        "eta": "15s remaining"
      },
      {
        "title": "Building artifacts",
        "status": "pending"
      },
      {
        "title": "Publishing",
        "status": "pending"
      }
    ],
    "show_details": true,
    "cancelable": true
  }
}
```

**Progress Variants**
- **Linear**: Traditional progress bar with percentage
- **Circular**: Circular progress indicator for compact spaces
- **Multi-Step**: Step-by-step process visualization
- **Indeterminate**: Spinner for unknown duration tasks
- **Real-time**: Live updating with streaming data

#### Notification Component
Non-intrusive alerts and status updates.

**Rich Notifications**
```json
{
  "component": "notification",
  "props": {
    "type": "success",
    "title": "Deployment Successful",
    "message": "Your application has been deployed to production",
    "details": {
      "url": "https://app.example.com",
      "deployment_id": "deploy-abc123",
      "duration": "2m 34s"
    },
    "actions": [
      {"label": "View App", "action": "open_url", "style": "primary"},
      {"label": "View Logs", "action": "view_logs", "style": "secondary"}
    ],
    "persistent": true,
    "auto_dismiss": 10000
  }
}
```

**Notification Types**
- **Info**: General information and updates
- **Success**: Completed operations and achievements
- **Warning**: Important notices requiring attention
- **Error**: Failures and problems requiring action

### Visualization Components

#### Chart Component
Data visualization for metrics and analytics.

**Multi-Series Chart**
```json
{
  "component": "chart",
  "props": {
    "type": "line",
    "title": "Application Performance",
    "data": {
      "labels": ["00:00", "00:15", "00:30", "00:45", "01:00"],
      "series": [
        {
          "name": "Response Time",
          "data": [120, 145, 132, 167, 148],
          "color": "#3b82f6",
          "unit": "ms"
        },
        {
          "name": "CPU Usage",
          "data": [45, 52, 48, 61, 55],
          "color": "#ef4444",
          "unit": "%"
        }
      ]
    },
    "axes": {
      "x": {"label": "Time", "type": "time"},
      "y": {"label": "Value", "type": "linear"}
    },
    "interactive": true,
    "zoom": true,
    "export": ["png", "svg", "csv"]
  }
}
```

**Chart Types**
- **Line**: Time series and trend visualization
- **Bar**: Categorical data comparison
- **Scatter**: Correlation and distribution analysis
- **Pie**: Proportion and percentage display
- **Histogram**: Distribution visualization
- **Heatmap**: Matrix data representation

#### Status Grid Component
Dashboard-style status overview with metrics and indicators.

**System Status Dashboard**
```json
{
  "component": "status_grid",
  "props": {
    "title": "System Health",
    "cards": [
      {
        "title": "API Status",
        "status": "healthy",
        "primary_metric": "99.9%",
        "secondary_metrics": ["156ms avg", "2.3K req/min"],
        "trend": "up",
        "actions": ["view_logs", "run_diagnostics"],
        "details": {
          "uptime": "30d 14h 23m",
          "last_incident": "2 weeks ago"
        }
      },
      {
        "title": "Database",
        "status": "warning",
        "primary_metric": "78% disk",
        "secondary_metrics": ["45 connections", "1.2GB cache"],
        "trend": "up",
        "actions": ["optimize", "scale_up"],
        "alerts": ["Disk usage approaching 80% threshold"]
      },
      {
        "title": "Cache",
        "status": "healthy",
        "primary_metric": "94% hit",
        "secondary_metrics": ["12GB used", "Redis 6.2"],
        "trend": "stable"
      }
    ],
    "refresh_interval": 30000,
    "layout": "grid"
  }
}
```

### Layout Components

#### Container Component
Flexible layout container with responsive behavior.

**Advanced Layout**
```json
{
  "component": "container",
  "props": {
    "layout": "flex",
    "direction": "column",
    "gap": "medium",
    "padding": "large",
    "children": [
      {
        "component": "header",
        "props": {
          "title": "Project Overview",
          "subtitle": "Development environment status",
          "actions": ["refresh", "settings"]
        }
      },
      {
        "component": "grid",
        "props": {
          "columns": 2,
          "gap": "medium",
          "children": [
            {"component": "status_card", "props": {...}},
            {"component": "metric_chart", "props": {...}}
          ]
        }
      }
    ]
  }
}
```

#### Split Panel Component
Resizable panels for complex layouts.

**Code Review Interface**
```json
{
  "component": "split_panel",
  "props": {
    "orientation": "horizontal",
    "initial_sizes": [30, 70],
    "min_sizes": [200, 300],
    "resizable": true,
    "panels": [
      {
        "component": "file_tree",
        "props": {
          "root": "/project",
          "filter": "modified"
        }
      },
      {
        "component": "split_panel",
        "props": {
          "orientation": "vertical",
          "panels": [
            {
              "component": "diff_viewer",
              "props": {"file": "src/main.rs"}
            },
            {
              "component": "commit_form",
              "props": {...}
            }
          ]
        }
      }
    ]
  }
}
```

#### Tabs Component
Organized content with navigation.

**Multi-View Interface**
```json
{
  "component": "tabs",
  "props": {
    "active_tab": "overview",
    "closable": true,
    "reorderable": true,
    "tabs": [
      {
        "id": "overview",
        "label": "Overview",
        "icon": "dashboard",
        "content": {
          "component": "status_grid",
          "props": {...}
        }
      },
      {
        "id": "logs",
        "label": "Logs",
        "icon": "file-text",
        "badge": "23",
        "content": {
          "component": "log_viewer",
          "props": {...}
        }
      },
      {
        "id": "metrics",
        "label": "Metrics",
        "icon": "bar-chart",
        "content": {
          "component": "metrics_dashboard",
          "props": {...}
        }
      }
    ]
  }
}
```

## Specialized Components

### Terminal Integration Components

#### Log Viewer Component
Optimized display for log files and streaming output.

**Advanced Log Viewer**
```json
{
  "component": "log_viewer",
  "props": {
    "source": "build_output",
    "auto_scroll": true,
    "line_numbers": true,
    "search": true,
    "filters": {
      "levels": ["error", "warn", "info", "debug"],
      "active_levels": ["error", "warn", "info"]
    },
    "syntax_highlighting": "json",
    "max_lines": 10000,
    "actions": ["download", "clear", "search"],
    "format": {
      "timestamp": "HH:mm:ss",
      "show_level": true,
      "color_by_level": true
    }
  }
}
```

#### Command History Component
Interactive command history with search and rerun capabilities.

**Smart History**
```json
{
  "component": "command_history",
  "props": {
    "commands": [
      {
        "command": "git status",
        "timestamp": "2023-12-01T10:30:00Z",
        "exit_code": 0,
        "duration": "0.12s",
        "working_directory": "/project",
        "frequency": 15
      }
    ],
    "search": {
      "enabled": true,
      "fuzzy": true,
      "filters": ["command", "directory", "exit_code"]
    },
    "grouping": "by_day",
    "actions": ["rerun", "edit", "copy", "bookmark"]
  }
}
```

### Developer Tool Components

#### Diff Viewer Component
Side-by-side or unified diff display with syntax highlighting.

**Rich Diff Display**
```json
{
  "component": "diff_viewer",
  "props": {
    "mode": "side_by_side",
    "old_content": "function hello() {\n  console.log('Hello');\n}",
    "new_content": "function hello(name = 'World') {\n  console.log(`Hello, ${name}!`);\n}",
    "language": "javascript",
    "line_numbers": true,
    "word_wrap": false,
    "context_lines": 3,
    "actions": ["accept", "reject", "edit"],
    "annotations": [
      {
        "line": 2,
        "type": "comment",
        "text": "Added default parameter and template literal"
      }
    ]
  }
}
```

#### Code Editor Component
Embedded code editing with syntax highlighting and basic editing features.

**Inline Editor**
```json
{
  "component": "code_editor",
  "props": {
    "content": "#!/bin/bash\necho 'Hello, World!'",
    "language": "bash",
    "theme": "dark",
    "line_numbers": true,
    "minimap": false,
    "read_only": false,
    "auto_complete": true,
    "actions": ["save", "format", "validate"]
  }
}
```

## Component Interaction Patterns

### Keyboard Navigation
Every component supports keyboard navigation with consistent patterns:

- **Global**: `Ctrl+P` (command palette), `Ctrl+/` (help), `Esc` (cancel/close)
- **Navigation**: `Tab/Shift+Tab` (focus), `Arrow keys` (directional movement)
- **Selection**: `Space` (toggle), `Ctrl+A` (select all), `Ctrl+Click` (multi-select)
- **Actions**: `Enter` (primary action), `Ctrl+Enter` (secondary action)

### Mouse and Touch
Rich mouse interaction with touch support:

- **Click**: Primary action activation
- **Double-click**: Quick action (edit, expand)
- **Right-click**: Context menu
- **Drag**: Reordering, resizing, selection
- **Hover**: Tooltips and previews

### Accessibility
Full accessibility support throughout:

- **Screen Readers**: Semantic markup and ARIA labels
- **High Contrast**: Accessible color schemes
- **Keyboard Only**: Complete keyboard navigation
- **Focus Management**: Logical focus flow and indicators

## Customization and Theming

### Theme System
Comprehensive theming with built-in and custom themes:

```json
{
  "theme": {
    "name": "custom_dark",
    "colors": {
      "background": "#1e1e1e",
      "foreground": "#d4d4d4",
      "accent": "#007acc",
      "success": "#4caf50",
      "warning": "#ff9800",
      "error": "#f44336"
    },
    "typography": {
      "font_family": "JetBrains Mono",
      "font_size": 14,
      "line_height": 1.4
    },
    "spacing": {
      "small": 4,
      "medium": 8,
      "large": 16
    }
  }
}
```

### Component Styling
Per-component style overrides:

```json
{
  "component": "table",
  "props": {...},
  "style": {
    "header": {
      "background": "#2d3748",
      "text_color": "#e2e8f0",
      "font_weight": "bold"
    },
    "row": {
      "hover_background": "#4a5568",
      "selected_background": "#2b6cb0"
    }
  }
}
```

This comprehensive component library provides the building blocks for creating rich, interactive CLI applications while maintaining the speed and efficiency that command-line users expect.
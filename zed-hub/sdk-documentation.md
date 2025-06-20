# SDK Documentation

**Comprehensive developer tools and libraries for building rich CLI interfaces**

## Overview

The Hub SDK provides everything developers need to transform their command-line applications into rich, interactive experiences. The SDK is designed around simplicity, performance, and progressive enhancement—your CLI tools continue to work exactly as before while gaining powerful new capabilities when connected to The Hub.

## SDK Design Philosophy

### 1. Zero Breaking Changes
- Existing CLI behavior remains completely unchanged
- SDK integration is purely additive
- Fallback to traditional CLI when The Hub unavailable
- No dependencies required for basic CLI functionality

### 2. Progressive Enhancement
- Start with basic protocol integration
- Add rich UI components incrementally
- Opt-in to advanced features as needed
- Backward compatibility across all versions

### 3. Developer Experience First
- Minimal boilerplate code
- Intuitive APIs with sensible defaults
- Comprehensive examples and documentation
- Rich debugging and development tools

### 4. Performance by Default
- Efficient protocol implementation
- Minimal overhead when protocol unavailable
- Asynchronous operations by default
- Memory-conscious design patterns

## Language SDKs

### Rust SDK

**Installation**
```toml
[dependencies]
the-hub = "1.0"

# Optional features
the-hub = { version = "1.0", features = ["ai", "forms", "charts"] }
```

**Basic Integration**
```rust
use the_hub::{Hub, Component, Result};

fn main() -> Result<()> {
    let mut hub = Hub::connect()?;
    
    // Your existing CLI logic
    let files = scan_project_files()?;
    
    // Rich UI when connected to The Hub
    if hub.is_connected() {
        hub.show_table()
            .headers(&["File", "Size", "Modified"])
            .rows(files.iter().map(|f| vec![
                f.path.display().to_string(),
                format_size(f.size),
                format_time(f.modified)
            ]))
            .send()?;
    } else {
        // Traditional CLI output
        for file in files {
            println!("{}", file.path.display());
        }
    }
    
    Ok(())
}
```

**Advanced Components**
```rust
use the_hub::components::*;

// Progress tracking
let progress = hub.progress()
    .total(100)
    .message("Processing files...")
    .show_eta(true)
    .create()?;

for (i, file) in files.iter().enumerate() {
    process_file(file)?;
    progress.update(i + 1, format!("Processed {}", file.name))?;
}

progress.complete("All files processed")?;

// Interactive forms
let response = hub.form()
    .title("Deployment Configuration")
    .field(TextField::new("environment")
        .label("Target Environment")
        .placeholder("production")
        .required(true))
    .field(SelectField::new("region")
        .label("AWS Region")
        .options(&["us-east-1", "us-west-2", "eu-west-1"])
        .default("us-east-1"))
    .field(CheckboxField::new("rollback")
        .label("Enable automatic rollback")
        .default(true))
    .submit_button("Deploy")
    .cancel_button("Cancel")
    .show()?;

if response.action == "Deploy" {
    let env = response.get_string("environment")?;
    let region = response.get_string("region")?;
    let rollback = response.get_bool("rollback")?;
    
    deploy_application(env, region, rollback)?;
}
```

### Python SDK

**Installation**
```bash
pip install the-hub
```

**Basic Integration**
```python
from the_hub import Hub, Table, Progress
import os

def main():
    hub = Hub.connect()
    
    # Scan directory
    files = []
    for root, dirs, filenames in os.walk('.'):
        for filename in filenames:
            path = os.path.join(root, filename)
            stat = os.stat(path)
            files.append({
                'path': path,
                'size': stat.st_size,
                'modified': stat.st_mtime
            })
    
    # Rich UI display
    if hub.is_connected():
        table = Table(headers=['File', 'Size', 'Modified'])
        for file in files:
            table.add_row([
                file['path'],
                format_size(file['size']),
                format_time(file['modified'])
            ])
        hub.show(table)
    else:
        # Traditional output
        for file in files:
            print(file['path'])

if __name__ == '__main__':
    main()
```

**Async/Await Support**
```python
import asyncio
from the_hub import AsyncHub

async def process_files():
    hub = await AsyncHub.connect()
    
    progress = await hub.progress(
        total=len(files),
        message="Processing files..."
    )
    
    async for i, file in enumerate(files):
        await process_file_async(file)
        await progress.update(i + 1)
    
    await progress.complete()

asyncio.run(process_files())
```

### JavaScript/TypeScript SDK

**Installation**
```bash
npm install the-hub
# or
yarn add the-hub
```

**Basic Integration**
```typescript
import { Hub, Table, Form } from 'the-hub';

async function main() {
    const hub = await Hub.connect();
    
    // Get project information
    const packageJson = JSON.parse(fs.readFileSync('package.json', 'utf8'));
    const dependencies = Object.entries(packageJson.dependencies || {});
    
    if (hub.isConnected()) {
        // Rich dependency table
        await hub.showTable({
            headers: ['Package', 'Version', 'Latest'],
            rows: await Promise.all(dependencies.map(async ([name, version]) => {
                const latest = await getLatestVersion(name);
                return [
                    name,
                    version,
                    latest,
                    version === latest ? '✓' : '⚠️'
                ];
            })),
            actions: {
                'update': 'Update All',
                'audit': 'Security Audit'
            }
        });
    } else {
        // Traditional CLI output
        dependencies.forEach(([name, version]) => {
            console.log(`${name}: ${version}`);
        });
    }
}
```

**React-like Component API**
```typescript
import { Component, useState, useEffect } from 'the-hub/react';

function DependencyManager() {
    const [dependencies, setDependencies] = useState([]);
    const [loading, setLoading] = useState(true);
    
    useEffect(() => {
        loadDependencies().then(deps => {
            setDependencies(deps);
            setLoading(false);
        });
    }, []);
    
    if (loading) {
        return <Progress message="Loading dependencies..." />;
    }
    
    return (
        <Table
            headers={['Package', 'Version', 'Status']}
            rows={dependencies.map(dep => [
                dep.name,
                dep.version,
                dep.isOutdated ? '⚠️ Outdated' : '✓ Current'
            ])}
            onRowClick={(row) => showPackageDetails(row[0])}
        />
    );
}

// Render the component
hub.render(<DependencyManager />);
```

### Go SDK

**Installation**
```bash
go get github.com/the-hub/sdk-go
```

**Basic Integration**
```go
package main

import (
    "fmt"
    "log"
    "github.com/the-hub/sdk-go"
)

func main() {
    hub, err := hub.Connect()
    if err != nil {
        log.Fatal(err)
    }
    defer hub.Close()
    
    files, err := scanFiles(".")
    if err != nil {
        log.Fatal(err)
    }
    
    if hub.IsConnected() {
        // Rich table display
        table := hub.NewTable().
            Headers("File", "Size", "Type").
            Sortable(true).
            Filterable(true)
        
        for _, file := range files {
            table.AddRow(file.Name, formatSize(file.Size), file.Type)
        }
        
        if err := hub.Show(table); err != nil {
            log.Printf("Failed to show table: %v", err)
        }
    } else {
        // Traditional CLI output
        for _, file := range files {
            fmt.Println(file.Name)
        }
    }
}
```

**Concurrent Operations**
```go
func processFiles(files []File) error {
    hub, err := hub.Connect()
    if err != nil {
        return err
    }
    
    progress := hub.NewProgress().
        Total(len(files)).
        Message("Processing files...").
        ShowETA(true)
    
    // Process files concurrently
    semaphore := make(chan struct{}, 10) // Limit concurrency
    var wg sync.WaitGroup
    
    for i, file := range files {
        wg.Add(1)
        go func(i int, file File) {
            defer wg.Done()
            semaphore <- struct{}{} // Acquire
            defer func() { <-semaphore }() // Release
            
            if err := processFile(file); err != nil {
                progress.Error(fmt.Sprintf("Failed to process %s: %v", file.Name, err))
            } else {
                progress.Update(i+1, fmt.Sprintf("Processed %s", file.Name))
            }
        }(i, file)
    }
    
    wg.Wait()
    progress.Complete("All files processed")
    return nil
}
```

## Advanced SDK Features

### 1. AI Integration

**Context Sharing**
```rust
use zed_hub::ai::*;

fn main() -> Result<()> {
    let mut hub = Hub::connect()?;
    
    // Share context with AI
    hub.ai_context()
        .command("git status")
        .working_directory(std::env::current_dir()?)
        .project_type("rust")
        .recent_commands(&["cargo build", "cargo test"])
        .share()?;
    
    // Request AI assistance
    let suggestion = hub.ai_suggest()
        .prompt("Help me understand this git status output")
        .confidence_threshold(0.8)
        .request()?;
    
    if let Some(suggestion) = suggestion {
        hub.show_notification()
            .title("AI Suggestion")
            .message(&suggestion.text)
            .actions(&[
                ("apply", "Apply Suggestion"),
                ("dismiss", "Dismiss")
            ])
            .send()?;
    }
    
    Ok(())
}
```

**Custom AI Prompts**
```python
from zed_hub.ai import AIAssistant

def analyze_logs():
    hub = Hub.connect()
    ai = AIAssistant(hub)
    
    # Read log file
    with open('app.log', 'r') as f:
        logs = f.read()
    
    # AI analysis
    analysis = ai.analyze_text(
        text=logs,
        prompt="Identify errors and performance issues in these logs",
        context={
            'file_type': 'application_log',
            'time_range': 'last_24_hours',
            'app_name': 'web_server'
        }
    )
    
    if analysis.issues:
        hub.show_status_grid([
            {
                'title': 'Critical Issues',
                'status': 'error',
                'count': len(analysis.critical_issues),
                'items': analysis.critical_issues
            },
            {
                'title': 'Warnings',
                'status': 'warning', 
                'count': len(analysis.warnings),
                'items': analysis.warnings
            },
            {
                'title': 'Performance',
                'status': 'info',
                'metrics': analysis.performance_metrics
            }
        ])
```

### 2. Real-time Updates

**Streaming Data**
```rust
use tokio_stream::StreamExt;

async fn monitor_build() -> Result<()> {
    let mut hub = Hub::connect().await?;
    
    let logs = hub.stream_component()
        .component_type("log_viewer")
        .title("Build Output")
        .create().await?;
    
    let mut build_process = start_build_process().await?;
    
    while let Some(line) = build_process.stdout.next().await {
        logs.append_line(LogLine {
            content: line,
            level: detect_log_level(&line),
            timestamp: Utc::now(),
        }).await?;
    }
    
    let exit_code = build_process.wait().await?;
    
    if exit_code.success() {
        logs.set_status("success", "Build completed successfully").await?;
    } else {
        logs.set_status("error", "Build failed").await?;
    }
    
    Ok(())
}
```

**Live Metrics Dashboard**
```typescript
import { Hub, MetricsDashboard } from 'zed-hub';

async function monitorSystem() {
    const hub = await Hub.connect();
    
    const dashboard = hub.createDashboard({
        title: 'System Monitoring',
        refreshInterval: 1000, // 1 second
        metrics: [
            {
                name: 'CPU Usage',
                type: 'gauge',
                unit: '%',
                max: 100
            },
            {
                name: 'Memory Usage', 
                type: 'gauge',
                unit: 'MB',
                max: 16384
            },
            {
                name: 'Network I/O',
                type: 'chart',
                unit: 'Mbps'
            }
        ]
    });
    
    setInterval(async () => {
        const stats = await getSystemStats();
        
        dashboard.updateMetrics({
            'CPU Usage': stats.cpu,
            'Memory Usage': stats.memory,
            'Network I/O': stats.network
        });
    }, 1000);
}
```

### 3. Custom Components

**Component Development**
```rust
use zed_hub::component::{Component, ComponentProps, RenderContext};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct GitStatusProps {
    repository_path: String,
    show_untracked: bool,
    show_ignored: bool,
}

pub struct GitStatusComponent;

impl Component for GitStatusComponent {
    type Props = GitStatusProps;
    
    fn render(&self, props: &Self::Props, ctx: &mut RenderContext) -> Result<()> {
        let repo = git2::Repository::open(&props.repository_path)?;
        let statuses = repo.statuses(None)?;
        
        let mut modified = Vec::new();
        let mut untracked = Vec::new();
        let mut staged = Vec::new();
        
        for entry in statuses.iter() {
            let status = entry.status();
            let path = entry.path().unwrap_or("unknown");
            
            if status.is_wt_modified() {
                modified.push(path);
            }
            if status.is_wt_new() && props.show_untracked {
                untracked.push(path);
            }
            if status.is_index_modified() || status.is_index_new() {
                staged.push(path);
            }
        }
        
        ctx.render_sections(&[
            Section::new("Staged Changes")
                .items(&staged)
                .color("green")
                .actions(&["unstage", "commit"]),
            Section::new("Modified Files")
                .items(&modified)
                .color("yellow")
                .actions(&["stage", "diff", "discard"]),
            Section::new("Untracked Files")
                .items(&untracked)
                .color("blue")
                .actions(&["stage", "ignore"])
                .visible(props.show_untracked),
        ])?;
        
        Ok(())
    }
}

// Register the component
fn main() -> Result<()> {
    let mut hub = Hub::connect()?;
    
    hub.register_component::<GitStatusComponent>("git_status")?;
    
    hub.show_component("git_status", GitStatusProps {
        repository_path: ".".to_string(),
        show_untracked: true,
        show_ignored: false,
    })?;
    
    Ok(())
}
```

## SDK Utilities and Tools

### 1. Development Tools

**Debug Mode**
```rust
use zed_hub::debug::*;

fn main() -> Result<()> {
    // Enable debug logging
    Hub::enable_debug_mode(DebugLevel::Verbose)?;
    
    let mut hub = Hub::connect()?;
    
    // Debug protocol messages
    hub.debug_protocol_messages(true);
    
    // Performance profiling
    let _profile = hub.start_profiling("component_render");
    
    hub.show_table()
        .headers(&["Column 1", "Column 2"])
        .rows(&[["Data 1", "Data 2"]])
        .send()?;
    
    // Profile results automatically logged
    
    Ok(())
}
```

**Testing Framework**
```rust
use zed_hub::testing::*;

#[test]
fn test_table_component() {
    let mut mock_hub = MockHub::new();
    
    // Configure expected protocol messages
    mock_hub.expect_table()
        .with_headers(&["Name", "Value"])
        .with_rows(&[["test", "123"]])
        .times(1);
    
    // Run your CLI function
    let result = display_data(&mock_hub);
    
    assert!(result.is_ok());
    mock_hub.verify(); // Verify all expectations met
}

#[integration_test]
async fn test_full_workflow() {
    let test_hub = TestHub::start().await?;
    
    // Run CLI command in test environment
    let output = test_hub.run_command("my-cli", &["--format", "table"]).await?;
    
    // Verify UI components were created
    assert_eq!(output.components.len(), 1);
    assert_eq!(output.components[0].component_type, "table");
    
    // Simulate user interaction
    let response = test_hub.click_button("export").await?;
    assert_eq!(response.action, "export_data");
}
```

### 2. Build Integration

**Cargo Integration**
```toml
[package.metadata.zed-hub]
components = ["table", "progress", "form"]
ai_features = true
custom_components = ["src/components/git_status.rs"]

[build-dependencies]
zed-hub-build = "1.0"
```

**Build Script**
```rust
// build.rs
use zed_hub_build::*;

fn main() {
    // Generate component bindings
    ComponentGenerator::new()
        .input_dir("src/components")
        .output_file("src/generated/components.rs")
        .generate()
        .expect("Failed to generate components");
    
    // Validate protocol messages
    ProtocolValidator::new()
        .schema_dir("schemas/")
        .validate_messages()
        .expect("Protocol validation failed");
}
```

**NPM Integration**
```json
{
  "scripts": {
    "build": "zed-hub build",
    "dev": "zed-hub dev --watch",
    "test": "zed-hub test"
  },
  "zed-hub": {
    "components": ["table", "chart", "form"],
    "entry": "src/cli.js",
    "output": "dist/cli.js"
  }
}
```

### 3. Documentation Generation

**Auto-generated Docs**
```rust
use zed_hub::docs::*;

/// Display project files in a table
/// 
/// # Zed-Hub Components
/// - Table with sortable columns
/// - File size formatting
/// - Action buttons for each file
/// 
/// # AI Integration
/// - Suggests file organization improvements
/// - Detects large files that should be ignored
/// 
/// # Examples
/// ```
/// my-cli list --format table
/// my-cli list --sort size --descending
/// ```
#[zed_hub_command]
pub fn list_files(args: ListArgs) -> Result<()> {
    // Implementation
}
```

## Migration Strategies

### 1. Existing CLI Migration

**Phase 1: Basic Integration**
```rust
// Minimal changes to existing CLI
fn main() -> Result<()> {
    let hub = Hub::try_connect(); // Non-blocking attempt
    
    let files = scan_files()?;
    
    match hub {
        Some(hub) => {
            // Rich UI mode
            display_files_rich(&hub, &files)?;
        },
        None => {
            // Traditional CLI mode (unchanged)
            display_files_traditional(&files)?;
        }
    }
    
    Ok(())
}
```

**Phase 2: Enhanced Features**
```rust
// Add rich interactions
fn display_files_rich(hub: &Hub, files: &[File]) -> Result<()> {
    let response = hub.show_table()
        .headers(&["File", "Size", "Modified"])
        .rows(files.iter().map(format_file_row))
        .actions(&[
            ("open", "Open"),
            ("delete", "Delete"),
            ("properties", "Properties")
        ])
        .show_and_wait()?;
    
    match response.action.as_str() {
        "open" => open_files(&response.selected_items)?,
        "delete" => delete_files(&response.selected_items)?,
        "properties" => show_properties(&response.selected_items)?,
        _ => {}
    }
    
    Ok(())
}
```

### 2. Gradual Feature Adoption

**Feature Flags**
```rust
use zed_hub::features::*;

fn main() -> Result<()> {
    let hub = Hub::connect()?;
    
    // Check feature availability
    if hub.supports_feature(Feature::InteractiveTables) {
        show_interactive_table(&hub)?;
    } else if hub.supports_feature(Feature::BasicTables) {
        show_basic_table(&hub)?;
    } else {
        show_text_output()?;
    }
    
    Ok(())
}
```

This comprehensive SDK documentation provides developers with everything they need to integrate rich UI capabilities into their CLI applications while maintaining backward compatibility and following best practices.
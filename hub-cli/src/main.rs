//! Demo CLI tool that demonstrates Hub protocol communication
//!
//! This is a simple CLI tool that connects to The Hub server and demonstrates
//! how CLI tools can send rich UI components to be displayed in the terminal.

use std::env;
use std::time::Duration;
use anyhow::Result;
use tokio::time::sleep;

// Import our Hub protocol types
use hub_protocol::{
    MessageEnvelope, MessageType, MessagePayload,
    UiMessagePayload, ProgressComponent, ProgressProps,
    TableComponent, TableProps, TableHeader, TableRow,
    HubClientBuilder, SessionCapabilities
};

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).unwrap_or(&"demo".to_string()).clone();
    
    println!("🔧 Hub Demo CLI Tool");
    println!("🎯 Command: {}", command);
    
    // Try to connect to Hub server
    println!("🔌 Connecting to Hub server...");
    
    let mut client = match HubClientBuilder::default().connect().await {
        Ok(client) => {
            println!("✅ Connected to Hub server!");
            client
        }
        Err(e) => {
            println!("❌ Failed to connect to Hub server: {}", e);
            println!("🔧 Running in traditional CLI mode...");
            run_traditional_mode(&command).await?;
            return Ok(());
        }
    };
    
    // Start Hub session
    println!("🚀 Starting Hub session...");
    client.start_session(
        command.clone(),
        args[1..].iter().map(|s| s.to_string()).collect(),
        env::current_dir()?.to_string_lossy().to_string(),
    ).await?;
    
    // Demonstrate different Hub components based on command
    match command.as_str() {
        "progress" => demo_progress(&mut client).await?,
        "table" => demo_table(&mut client).await?,
        "files" => demo_file_tree(&mut client).await?,
        _ => demo_mixed(&mut client).await?,
    }
    
    // End session
    println!("🏁 Ending Hub session...");
    client.end_session(0, 1000, Some("Demo completed successfully".to_string())).await?;
    
    println!("✅ Demo completed!");
    Ok(())
}

/// Demonstrate progress component
async fn demo_progress(client: &mut hub_protocol::HubClient) -> Result<()> {
    println!("📊 Demonstrating progress component...");
    
    for i in 0..=10 {
        let progress = i * 10;
        client.show_progress(
            progress,
            100,
            format!("Processing item {} of 10...", i + 1),
        ).await?;
        
        sleep(Duration::from_millis(500)).await;
    }
    
    Ok(())
}

/// Demonstrate table component
async fn demo_table(client: &mut hub_protocol::HubClient) -> Result<()> {
    println!("📋 Demonstrating table component...");
    
    let headers = vec![
        "Name".to_string(),
        "Status".to_string(),
        "Progress".to_string(),
        "Time".to_string(),
    ];
    
    let rows = vec![
        vec!["Build Project".to_string(), "✅ Complete".to_string(), "100%".to_string(), "2.3s".to_string()],
        vec!["Run Tests".to_string(), "🟡 Running".to_string(), "75%".to_string(), "1.8s".to_string()],
        vec!["Deploy".to_string(), "⏳ Pending".to_string(), "0%".to_string(), "-".to_string()],
        vec!["Notify Team".to_string(), "⏳ Pending".to_string(), "0%".to_string(), "-".to_string()],
    ];
    
    client.show_table(headers, rows).await?;
    
    // Simulate some updates
    sleep(Duration::from_secs(2)).await;
    
    let updated_rows = vec![
        vec!["Build Project".to_string(), "✅ Complete".to_string(), "100%".to_string(), "2.3s".to_string()],
        vec!["Run Tests".to_string(), "✅ Complete".to_string(), "100%".to_string(), "3.1s".to_string()],
        vec!["Deploy".to_string(), "🟡 Running".to_string(), "45%".to_string(), "5.2s".to_string()],
        vec!["Notify Team".to_string(), "⏳ Pending".to_string(), "0%".to_string(), "-".to_string()],
    ];
    
    client.show_table(vec!["Name".to_string(), "Status".to_string(), "Progress".to_string(), "Time".to_string()], updated_rows).await?;
    
    Ok(())
}

/// Demonstrate file tree component
async fn demo_file_tree(client: &mut hub_protocol::HubClient) -> Result<()> {
    println!("📁 Demonstrating file tree component...");
    
    use hub_protocol::{FileEntry, FileEntryType};
    use chrono::{DateTime, Utc};
    
    let entries = vec![
        FileEntry {
            path: "src".to_string(),
            entry_type: FileEntryType::Directory,
            size: None,
            modified: None,
            status: None,
            actions: vec![],
            children: Some(vec![
                FileEntry {
                    path: "src/main.rs".to_string(),
                    entry_type: FileEntryType::File,
                    size: Some(1234),
                    modified: Some("2024-01-15T10:30:00Z".parse::<DateTime<Utc>>().unwrap()),
                    status: None,
                    actions: vec![],
                    children: None,
                    expanded: false,
                },
                FileEntry {
                    path: "src/lib.rs".to_string(),
                    entry_type: FileEntryType::File,
                    size: Some(567),
                    modified: Some("2024-01-15T09:15:00Z".parse::<DateTime<Utc>>().unwrap()),
                    status: None,
                    actions: vec![],
                    children: None,
                    expanded: false,
                },
            ]),
            expanded: true,
        },
        FileEntry {
            path: "Cargo.toml".to_string(),
            entry_type: FileEntryType::File,
            size: Some(890),
            modified: Some("2024-01-15T08:45:00Z".parse::<DateTime<Utc>>().unwrap()),
            status: None,
            actions: vec![],
            children: None,
            expanded: false,
        },
        FileEntry {
            path: "README.md".to_string(),
            entry_type: FileEntryType::File,
            size: Some(2345),
            modified: Some("2024-01-14T16:20:00Z".parse::<DateTime<Utc>>().unwrap()),
            status: None,
            actions: vec![],
            children: None,
            expanded: false,
        },
    ];
    
    client.show_file_tree("Project Root".to_string(), entries).await?;
    
    Ok(())
}

/// Demonstrate mixed components
async fn demo_mixed(client: &mut hub_protocol::HubClient) -> Result<()> {
    println!("🎨 Demonstrating mixed components...");
    
    // Start with progress
    client.show_progress(0, 4, "Starting demonstration...".to_string()).await?;
    sleep(Duration::from_secs(1)).await;
    
    // Show a simple table
    client.show_progress(1, 4, "Generating data table...".to_string()).await?;
    let headers = vec!["Component".to_string(), "Status".to_string()];
    let rows = vec![
        vec!["Progress Bar".to_string(), "✅ Working".to_string()],
        vec!["Table View".to_string(), "✅ Working".to_string()],
        vec!["File Tree".to_string(), "✅ Working".to_string()],
    ];
    client.show_table(headers, rows).await?;
    sleep(Duration::from_secs(1)).await;
    
    // Update progress
    client.show_progress(3, 4, "Almost done...".to_string()).await?;
    sleep(Duration::from_secs(1)).await;
    
    // Final progress
    client.show_progress(4, 4, "Demonstration complete!".to_string()).await?;
    
    Ok(())
}

/// Run in traditional CLI mode (fallback when Hub is not available)
async fn run_traditional_mode(command: &str) -> Result<()> {
    println!("Running in traditional CLI mode...");
    
    match command {
        "progress" => {
            println!("Simulating progress...");
            for i in 0..=10 {
                println!("[{}{}] {}% - Processing item {} of 10...", 
                    "=".repeat(i), 
                    " ".repeat(10 - i), 
                    i * 10, 
                    i + 1
                );
                sleep(Duration::from_millis(500)).await;
            }
        }
        "table" => {
            println!("Task Status Report:");
            println!("┌─────────────┬─────────────┬──────────┬────────┐");
            println!("│ Name        │ Status      │ Progress │ Time   │");
            println!("├─────────────┼─────────────┼──────────┼────────┤");
            println!("│ Build       │ ✅ Complete │ 100%     │ 2.3s   │");
            println!("│ Tests       │ 🟡 Running  │ 75%      │ 1.8s   │");
            println!("│ Deploy      │ ⏳ Pending  │ 0%       │ -      │");
            println!("│ Notify      │ ⏳ Pending  │ 0%       │ -      │");
            println!("└─────────────┴─────────────┴──────────┴────────┘");
        }
        "files" => {
            println!("Project structure:");
            println!("📁 src/");
            println!("  📄 main.rs (1.2 KB)");
            println!("  📄 lib.rs (567 B)");
            println!("📄 Cargo.toml (890 B)");
            println!("📄 README.md (2.3 KB)");
        }
        _ => {
            println!("🎯 Hub Demo CLI");
            println!("Available commands: progress, table, files");
            println!("Usage: {} <command>", env::args().next().unwrap_or("demo".to_string()));
        }
    }
    
    Ok(())
}
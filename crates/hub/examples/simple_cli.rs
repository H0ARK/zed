//! Bare-bones CLI tool that connects to Hub
//! Usage: cargo run --example simple_cli -p hub -- "ls -la"

use hub_protocol::*;
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::Result;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let command = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        "echo Hello from CLI!".to_string()
    };
    
    println!("🔌 Connecting to Hub server...");
    
    // Connect to Hub
    let mut stream = TcpStream::connect("127.0.0.1:8765").await?;
    println!("✅ Connected to Hub!");
    
    // Create session start message
    let session_id = uuid::Uuid::new_v4().to_string();
    let parts: Vec<&str> = command.split_whitespace().collect();
    let (cmd, args) = if parts.is_empty() {
        ("echo", vec!["Hello"])
    } else {
        (parts[0], parts[1..].iter().map(|s| s.to_string()).collect())
    };
    
    let capabilities = SessionCapabilities {
        ui_components: vec!["progress".to_string(), "table".to_string()],
        interactions: vec!["click".to_string()],
        ai_integration: false,
    };
    
    let session_msg = MessageEnvelope::session_start(
        session_id.clone(),
        1,
        cmd.to_string(),
        args,
        env::current_dir()?.to_string_lossy().to_string(),
        capabilities,
    );
    
    // Send session start
    let json = serde_json::to_string(&session_msg)?;
    stream.write_all(json.as_bytes()).await?;
    println!("📤 Sent session start for command: {}", command);
    
    // Send progress update
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    let progress_msg = MessageEnvelope::progress(
        session_id.clone(),
        2,
        1,
        3,
        "Starting command execution".to_string(),
    );
    
    let progress_json = serde_json::to_string(&progress_msg)?;
    stream.write_all(progress_json.as_bytes()).await?;
    println!("📤 Sent progress update");
    
    // Simulate command execution
    println!("⚡ Executing command: {}", command);
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .output()
        .await?;
    
    let result = String::from_utf8_lossy(&output.stdout);
    println!("📋 Command output:\n{}", result);
    
    // Send completion
    let completion_msg = MessageEnvelope::session_end(
        session_id.clone(),
        3,
        output.status.code().unwrap_or(-1),
        1000, // 1 second duration
        Some(format!("Command completed with {} bytes of output", result.len())),
    );
    
    let completion_json = serde_json::to_string(&completion_msg)?;
    stream.write_all(completion_json.as_bytes()).await?;
    println!("📤 Sent completion message");
    
    // Wait for response
    let mut buffer = vec![0; 4096];
    match stream.read(&mut buffer).await {
        Ok(n) if n > 0 => {
            let response = String::from_utf8_lossy(&buffer[..n]);
            println!("📨 Received response from Hub: {}", response);
        },
        _ => println!("⚠️  No response from Hub"),
    }
    
    println!("✅ CLI session complete!");
    Ok(())
}
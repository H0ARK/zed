//! Standalone bare-bones Hub client
//! Usage: cargo run --bin simple_hub_client

use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use serde_json::json;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let command = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        "echo Hello Hub!".to_string()
    };
    
    println!("🔌 Connecting to Hub server...");
    
    // Connect to Hub
    let mut stream = TcpStream::connect("127.0.0.1:8765").await?;
    println!("✅ Connected to Hub!");
    
    // Send session start message
    let session_start = json!({
        "version": "1.0",
        "type": "control",
        "session_id": "test-session-123",
        "sequence": 1,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "payload": {
            "action": "session_start",
            "command": command.split_whitespace().next().unwrap_or("echo"),
            "args": command.split_whitespace().skip(1).collect::<Vec<_>>(),
            "cwd": env::current_dir()?.to_string_lossy(),
            "capabilities": {
                "ui_components": ["progress", "table"],
                "interactions": ["click"],
                "ai_integration": false
            }
        }
    });
    
    let json_str = serde_json::to_string(&session_start)?;
    stream.write_all(json_str.as_bytes()).await?;
    println!("📤 Sent session start for command: {}", command);
    
    // Send progress update
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    let progress = json!({
        "version": "1.0",
        "type": "ui_message",
        "session_id": "test-session-123",
        "sequence": 2,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "payload": {
            "component": "progress",
            "props": {
                "current": 1,
                "total": 3,
                "message": "Executing command...",
                "show_percentage": true,
                "show_eta": true,
                "style": "bar"
            }
        }
    });
    
    let progress_str = serde_json::to_string(&progress)?;
    stream.write_all(progress_str.as_bytes()).await?;
    println!("📤 Sent progress update");
    
    // Execute the actual command
    println!("⚡ Executing: {}", command);
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .output()
        .await?;
    
    let result = String::from_utf8_lossy(&output.stdout);
    println!("📋 Command output:\n{}", result);
    
    // Send completion
    let completion = json!({
        "version": "1.0",
        "type": "control",
        "session_id": "test-session-123",
        "sequence": 3,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "payload": {
            "action": "session_end",
            "exit_code": output.status.code().unwrap_or(-1),
            "duration_ms": 1000,
            "summary": format!("Command completed: {} bytes output", result.len())
        }
    });
    
    let completion_str = serde_json::to_string(&completion)?;
    stream.write_all(completion_str.as_bytes()).await?;
    println!("📤 Sent completion");
    
    // Read response
    let mut response_buf = vec![0; 1024];
    match stream.read(&mut response_buf).await {
        Ok(n) if n > 0 => {
            let response = String::from_utf8_lossy(&response_buf[..n]);
            println!("📨 Hub response: {}", response);
        },
        _ => println!("⚠️  No response from Hub"),
    }
    
    println!("✅ Session complete!");
    Ok(())
}
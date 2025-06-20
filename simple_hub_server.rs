//! Standalone bare-bones Hub server
//! Usage: cargo run --bin simple_hub_server

use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting bare-bones Hub server on port 8765...");
    
    let listener = TcpListener::bind("127.0.0.1:8765").await?;
    println!("✅ Hub server listening on 127.0.0.1:8765");
    println!("📡 Waiting for CLI connections...\n");
    
    loop {
        let (mut socket, addr) = listener.accept().await?;
        println!("🔌 New connection from: {}", addr);
        
        tokio::spawn(async move {
            let mut buffer = vec![0; 4096];
            
            loop {
                match socket.read(&mut buffer).await {
                    Ok(0) => {
                        println!("🔌 Client {} disconnected", addr);
                        break;
                    },
                    Ok(n) => {
                        let data = String::from_utf8_lossy(&buffer[..n]);
                        println!("📨 Received from {}: {}", addr, data.trim());
                        
                        // Try to parse as JSON
                        if let Ok(msg) = serde_json::from_str::<Value>(&data) {
                            if let Some(msg_type) = msg.get("type") {
                                println!("✅ Parsed message type: {}", msg_type);
                                
                                // Echo back a simple response
                                let response = json!({
                                    "type": "response", 
                                    "status": "received",
                                    "echo": msg_type
                                });
                                
                                let response_json = serde_json::to_string(&response).unwrap();
                                if let Err(e) = socket.write_all(response_json.as_bytes()).await {
                                    println!("❌ Failed to send response: {}", e);
                                    break;
                                }
                                println!("📤 Sent response back to {}", addr);
                            }
                        } else {
                            println!("⚠️  Raw data from {}: {}", addr, data.trim());
                        }
                    },
                    Err(e) => {
                        println!("❌ Error reading from {}: {}", addr, e);
                        break;
                    }
                }
            }
        });
    }
}
//! Bare-bones Hub server that actually works
//! Usage: cargo run --example simple_server -p hub

use hub_protocol::*;
use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
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
                        
                        // Try to parse as JSON message
                        if let Ok(envelope) = serde_json::from_str::<MessageEnvelope>(&data) {
                            println!("✅ Parsed message: {:?} from session {}", 
                                   envelope.message_type, envelope.session_id);
                            
                            // Echo back a simple response
                            let response = MessageEnvelope::new(
                                MessageType::Response,
                                envelope.session_id,
                                envelope.sequence + 1,
                                MessagePayload::Response(ResponsePayload {
                                    interaction_id: "server_response".to_string(),
                                    action: "echo".to_string(),
                                    data: serde_json::json!({"status": "received"}),
                                }),
                            );
                            
                            let response_json = serde_json::to_string(&response).unwrap();
                            if let Err(e) = socket.write_all(response_json.as_bytes()).await {
                                println!("❌ Failed to send response: {}", e);
                                break;
                            }
                            println!("📤 Sent response back to {}", addr);
                        } else {
                            println!("⚠️  Invalid JSON from {}: {}", addr, data.trim());
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
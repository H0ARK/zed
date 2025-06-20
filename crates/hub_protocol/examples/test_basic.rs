// Quick test to see what Hub components actually work
use hub_protocol::messages::*;

fn main() -> anyhow::Result<()> {
    println!("🚀 Testing The Hub protocol...\n");
    
    // Test 1: Protocol messages
    println!("1. Testing protocol messages...");
    let message = MessageEnvelope::progress(
        "test-session".to_string(),
        1,
        50,
        100,
        "Testing progress".to_string(),
    );
    println!("   ✅ Created progress message: {:?}", message.message_type);
    println!("   📄 Message session: {}", message.session_id);
    println!("   🕐 Message timestamp: {}", message.timestamp);
    
    // Test 2: Error message
    let error_msg = MessageEnvelope::error(
        "test-session".to_string(),
        2,
        "TEST_ERROR".to_string(),
        "This is a test error".to_string(),
        None,
    );
    println!("\n2. Testing error message...");
    println!("   ✅ Created error message: {:?}", error_msg.message_type);
    
    // Test 3: Session start
    let capabilities = SessionCapabilities {
        ui_components: vec!["progress".to_string(), "table".to_string()],
        interactions: vec!["click".to_string()],
        ai_integration: true,
    };
    
    let session_msg = MessageEnvelope::session_start(
        "test-session".to_string(),
        3,
        "git".to_string(),
        vec!["status".to_string()],
        "/tmp".to_string(),
        capabilities,
    );
    println!("\n3. Testing session start...");
    println!("   ✅ Created session start message");
    
    // Test 4: JSON serialization
    println!("\n4. Testing JSON serialization...");
    let json = serde_json::to_string_pretty(&message)?;
    println!("   ✅ Serialized to JSON:");
    println!("{}", json);
    
    println!("\n🎉 Protocol layer works! Ready for real implementation.");
    
    Ok(())
}
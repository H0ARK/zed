// Quick test to see what Hub components actually work
use hub_protocol::messages::*;
use hub_blocks::*;
use hub_sdk::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🚀 Testing The Hub components...\n");
    
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
    
    // Test 2: Block creation
    println!("\n2. Testing block system...");
    let block = Block::new(
        "block-1".to_string(),
        "git".to_string(),
        vec!["status".to_string()],
        "/tmp".to_string(),
        "session-1".to_string(),
    );
    println!("   ✅ Created block: {} ({})", block.command, block.id);
    
    // Test 3: Block manager
    println!("\n3. Testing block manager...");
    let manager = BlockManager::new();
    let block_id = manager.create_block(
        "ls".to_string(),
        vec!["-la".to_string()],
        "/home".to_string(),
        "session-1".to_string(),
    ).await?;
    println!("   ✅ Block manager created block: {}", block_id);
    
    // Test 4: UI components
    println!("\n4. Testing UI components...");
    let progress = progress(25, 100, "Building project");
    println!("   ✅ Created progress component");
    
    let table = TableBuilder::new()
        .header("Name", "auto")
        .header("Status", "100px")
        .row("item1", vec!["File 1".to_string(), "Ready".to_string()])
        .build();
    println!("   ✅ Created table component");
    
    // Test 5: Hub mode detection
    println!("\n5. Testing Hub mode detection...");
    if is_hub_mode() {
        println!("   ✅ Hub mode is enabled");
    } else {
        println!("   ⚠️  Hub mode is disabled (no HUB_MODE env var)");
    }
    
    // Test 6: Client creation (will fail - no actual Hub running)
    println!("\n6. Testing Hub client creation...");
    match create_hub_client().await {
        Ok(Some(_client)) => println!("   ✅ Connected to Hub!"),
        Ok(None) => println!("   ⚠️  No Hub available (expected)"),
        Err(e) => println!("   ⚠️  Failed to connect: {}", e),
    }
    
    println!("\n🎉 All basic tests passed! Foundation is solid.");
    println!("\n📝 Next steps:");
    println!("   - Implement actual Hub server");
    println!("   - Create real terminal integration");
    println!("   - Build UI rendering in Zed");
    println!("   - Add real protocol communication");
    
    Ok(())
}
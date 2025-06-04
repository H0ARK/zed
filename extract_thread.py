#!/usr/bin/env python3
"""
Extract thread data from Zed's LMDB database for compression testing.
"""

import json
import sys
from pathlib import Path
import lmdb
import struct

def extract_thread_from_db(thread_id: str, db_path: str = None):
    """Extract a specific thread from Zed's LMDB database."""
    
    if db_path is None:
        db_path = Path.home() / "Library/Application Support/Zed/threads/threads-db.1.mdb"
    
    if not Path(db_path).exists():
        print(f"Database not found at: {db_path}")
        return None
    
    try:
        # Open the LMDB database
        env = lmdb.open(str(db_path), readonly=True, max_dbs=10)
        
        with env.begin() as txn:
            # Open the threads database
            threads_db = env.open_db(b'threads', txn=txn)
            
            # Convert thread ID to bytes (it's stored as a string key)
            thread_key = thread_id.encode('utf-8')
            
            # Get the thread data
            thread_data = txn.get(thread_key, db=threads_db)
            
            if thread_data is None:
                print(f"Thread {thread_id} not found in database")
                return None
            
            # Parse the JSON data
            thread_json = json.loads(thread_data.decode('utf-8'))
            return thread_json
            
    except Exception as e:
        print(f"Error reading database: {e}")
        return None

def analyze_thread_for_compression(thread_data):
    """Analyze thread data to understand its structure for compression testing."""
    
    if not thread_data:
        return
    
    print("=== Thread Analysis ===")
    print(f"Version: {thread_data.get('version', 'unknown')}")
    print(f"Summary: {thread_data.get('summary', 'No summary')[:100]}...")
    print(f"Updated: {thread_data.get('updated_at', 'unknown')}")
    
    messages = thread_data.get('messages', [])
    print(f"Total messages: {len(messages)}")
    
    # Analyze message types and sizes
    total_chars = 0
    user_messages = 0
    assistant_messages = 0
    tool_messages = 0
    
    for i, msg in enumerate(messages):
        role = msg.get('role', 'unknown')
        segments = msg.get('segments', [])
        
        if role == 'user':
            user_messages += 1
        elif role == 'assistant':
            assistant_messages += 1
        else:
            tool_messages += 1
        
        # Count characters in segments
        for segment in segments:
            if segment.get('type') == 'text':
                text = segment.get('text', '')
                total_chars += len(text)
            elif segment.get('type') == 'thinking':
                text = segment.get('text', '')
                total_chars += len(text)
        
        # Also count context
        context = msg.get('context', '')
        total_chars += len(context)
    
    print(f"Message breakdown:")
    print(f"  - User messages: {user_messages}")
    print(f"  - Assistant messages: {assistant_messages}")
    print(f"  - Tool messages: {tool_messages}")
    print(f"Total characters: {total_chars:,}")
    print(f"Estimated tokens: {total_chars // 4:,}")
    
    # Token usage info
    cumulative_usage = thread_data.get('cumulative_token_usage', {})
    if cumulative_usage:
        print(f"Cumulative token usage:")
        print(f"  - Input tokens: {cumulative_usage.get('input_tokens', 0):,}")
        print(f"  - Output tokens: {cumulative_usage.get('output_tokens', 0):,}")
        print(f"  - Total: {cumulative_usage.get('input_tokens', 0) + cumulative_usage.get('output_tokens', 0):,}")
    
    return {
        'total_messages': len(messages),
        'total_chars': total_chars,
        'estimated_tokens': total_chars // 4,
        'user_messages': user_messages,
        'assistant_messages': assistant_messages,
        'tool_messages': tool_messages,
        'cumulative_usage': cumulative_usage
    }

def create_test_messages_for_compression(thread_data):
    """Convert thread data into a format suitable for compression testing."""
    
    if not thread_data:
        return []
    
    messages = thread_data.get('messages', [])
    test_messages = []
    
    for msg in messages:
        role = msg.get('role', 'unknown')
        segments = msg.get('segments', [])
        context = msg.get('context', '')
        
        # Combine all text from segments
        text_content = []
        for segment in segments:
            if segment.get('type') == 'text':
                text_content.append(segment.get('text', ''))
            elif segment.get('type') == 'thinking':
                text_content.append(f"[Thinking: {segment.get('text', '')}]")
        
        # Combine with context
        full_content = '\n'.join(text_content)
        if context:
            full_content += f"\n[Context: {context}]"
        
        if full_content.strip():
            test_messages.append({
                'role': role,
                'content': full_content,
                'token_estimate': len(full_content) // 4
            })
    
    return test_messages

def main():
    # Get the most recent thread ID from navigation history
    nav_history_path = Path.home() / "Library/Application Support/Zed/agent-navigation-history.json"
    
    thread_id = None
    if nav_history_path.exists():
        try:
            with open(nav_history_path) as f:
                history = json.load(f)
                for entry in history:
                    if 'Thread' in entry:
                        thread_id = entry['Thread']
                        break
        except Exception as e:
            print(f"Error reading navigation history: {e}")
    
    if not thread_id:
        print("No thread ID found in navigation history")
        if len(sys.argv) > 1:
            thread_id = sys.argv[1]
        else:
            print("Usage: python extract_thread.py [thread_id]")
            return
    
    print(f"Extracting thread: {thread_id}")
    
    # Extract thread data
    thread_data = extract_thread_from_db(thread_id)
    
    if thread_data:
        # Analyze the thread
        analysis = analyze_thread_for_compression(thread_data)
        
        # Create test messages
        test_messages = create_test_messages_for_compression(thread_data)
        
        # Save for compression testing
        output_file = "extracted_thread_data.json"
        with open(output_file, 'w') as f:
            json.dump({
                'thread_id': thread_id,
                'analysis': analysis,
                'messages': test_messages,
                'raw_thread_data': thread_data
            }, f, indent=2)
        
        print(f"\nThread data saved to: {output_file}")
        print(f"Ready for compression testing with {len(test_messages)} messages")
        
        # Quick compression simulation
        if analysis and analysis['estimated_tokens'] > 32000:
            print(f"\n=== Compression Simulation ===")
            original_tokens = analysis['estimated_tokens']
            
            # Simulate different compression ratios
            for strategy, ratio in [("Recent Priority", 0.85), ("Smart Compression", 0.70), ("Aggressive", 0.50)]:
                compressed_tokens = int(original_tokens * ratio)
                savings = original_tokens - compressed_tokens
                savings_pct = (savings / original_tokens) * 100
                
                fits_limit = compressed_tokens <= 32000
                status = "✓" if fits_limit else "✗"
                
                print(f"  {status} {strategy}: {compressed_tokens:,} tokens ({ratio:.0%} preserved, {savings_pct:.1f}% savings)")

if __name__ == "__main__":
    main() 
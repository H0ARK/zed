# Zed Session Management Visual Guide

## Where to Find Session Management Features

### 1. Session Management Toggle Button

**Location**: Agent Panel Toolbar (Right side)

```
┌─ Agent Panel Toolbar ─────────────────────────────────────┐
│  ← Back    Title Area              [+] [👥] Sessions [⋮] │
│                                      │                    │
│                                      └─ NEW BUTTON HERE! │
└───────────────────────────────────────────────────────────┘
```

**What to Look For**:
- 👥 **Users icon** (medium size, more prominent than other buttons)
- **"Sessions" or "Single" label** next to the icon
- **Different styling**: 
  - When OFF: Outline style button, "Single" text in muted color
  - When ON: Filled/accent style button, "Sessions" text in accent color

**Keyboard Shortcut**: `Cmd+Alt+M` (macOS)

### 2. Session Management Panel (When Active)

**Location**: Below the toolbar, above the chat area

```
┌─ Agent Panel ─────────────────────────────────────────────┐
│  Toolbar with [👥] Sessions button (ACTIVE/FILLED)        │
├───────────────────────────────────────────────────────────┤
│  🤖 Multi-Agent Mode Active     ● (colored indicator)     │
├───────────────────────────────────────────────────────────┤
│  [+] Sessions:  [🤔 Main] [💤 Debug] [💬 Docs]  [▶]     │
│                    │        │         │        │         │
│                    └─ Status Icons ─────────┘  │         │
│                                                 └─ Coord  │
├───────────────────────────────────────────────────────────┤
│  🧠 AI Coordination (when expanded)                       │
│  Recent coordination history...                           │
├───────────────────────────────────────────────────────────┤
│  Chat area (regular agent conversation)                   │
│  ...                                                      │
└───────────────────────────────────────────────────────────┘
```

### 3. Visual Indicators

#### Session Status Icons:
- 🤔 **Thinking**: AI is processing your request
- 💬 **Responding**: AI is actively generating a response  
- ⏳ **Waiting**: Session is waiting for user input
- 💤 **Idle**: Session is inactive/ready
- ❌ **Error**: Session encountered an error

#### Button States:
- **Session Toggle OFF**: Outline button, small "Single" label
- **Session Toggle ON**: Filled/accent button, "Sessions" label in accent color
- **Active Session Tab**: Filled background
- **Inactive Session Tab**: Subtle background

### 4. Empty State (First Time)

When you first activate session management:

```
┌─ Welcome Screen ──────────────────────────────────────────┐
│              🚀 Agent-First Zed                           │
│                                                           │
│    Create multiple AI agent sessions and let the Main    │
│    AI coordinate their work. Each session can focus on   │
│    different aspects of your project.                    │
│                                                           │
│              [Create Your First Session]                 │
│              [Learn More About Coordination]             │
└───────────────────────────────────────────────────────────┘
```

## Troubleshooting "I Don't See It"

### Check These Things:

1. **Agent Panel is Open**: 
   - Press `Cmd+?` to toggle the Agent Panel
   - Look for the right sidebar with chat interface

2. **Look for the Users Icon**: 
   - Should be between the [+] and [⋮] buttons
   - Larger than other toolbar icons
   - Has text label "Single" or "Sessions"

3. **Icon Might Be Different**: 
   - Uses 👥 Users icon (not tree/list icon)
   - Medium size (larger than other toolbar icons)
   - Has accompanying text label

4. **Try the Keyboard Shortcut**:
   - Press `Cmd+Alt+M` while Agent Panel is focused
   - Should toggle the session management mode

5. **Look for Visual Changes**:
   - When enabled, you'll see "🤖 Multi-Agent Mode Active" banner
   - Button style changes from outline to filled
   - Text changes from "Single" to "Sessions"

### If Still Not Visible:

1. **Check Zed Version**: This feature requires the latest version with session management
2. **Restart Zed**: Close and reopen the application
3. **Reset Panel**: Try closing/reopening the Agent Panel with `Cmd+?`

## Quick Start Steps

1. **Open Agent Panel**: `Cmd+?`
2. **Find Session Button**: Look for 👥 with "Single" text in toolbar
3. **Activate Sessions**: Click the button or press `Cmd+Alt+M` 
4. **See the Change**: Button becomes filled, shows "Sessions" text
5. **Create First Session**: Click "Create Your First Session" in welcome screen
6. **Start Using**: You now have multi-agent session management!

## What Changes When Session Management is ON

- **Toolbar**: Session button becomes prominent and filled
- **Panel Header**: Shows "🤖 Multi-Agent Mode Active" banner  
- **Session Tabs**: Horizontal tabs appear below toolbar
- **Coordination**: Play button (▶) for coordinating multiple sessions
- **Chat Area**: Same as before, but now tied to active session

The core chat experience remains exactly the same - you're just now managing multiple conversations instead of one!
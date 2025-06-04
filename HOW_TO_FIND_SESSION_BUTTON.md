# How to Find the Session Management Button in Zed

## Quick Answer
Look for the **👥 UserGroup icon** with "Single"/"Sessions" text in the Agent Panel toolbar, between the [+] and [⋮] buttons.

## Step-by-Step Instructions

### 1. Open the Agent Panel
- Press `Cmd+?` (macOS) or `Ctrl+?` (Linux/Windows)
- Or click the Agent icon in the right sidebar
- You should see the chat interface

### 2. Look at the Toolbar
The toolbar is at the top of the Agent Panel and looks like this:
```
┌─ Agent Panel Toolbar ────────────────────────────────┐
│  [←] Thread Title          [+] [👥] Single [⋮]      │
└──────────────────────────────────────────────────────┘
```

### 3. Find the Session Button
Look for:
- **👥 Icon** (UserGroup icon - two people silhouettes)
- **Medium size** (larger than other toolbar icons)
- **Text label**: Shows "Single" when OFF, "Sessions" when ON
- **Location**: Between the "+" (new thread) and "⋮" (options menu) buttons

### 4. Visual States
**When Session Management is OFF:**
- Button has subtle/outline styling
- Shows "Single" text in muted color
- UserGroup icon is subtle

**When Session Management is ON:**
- Button becomes filled/prominent
- Shows "Sessions" text in accent color
- Icon becomes more prominent
- A banner appears: "🤖 Multi-Agent Mode Active"

### 5. Keyboard Shortcut
Press `Cmd+Alt+M` (macOS) while the Agent Panel is focused to toggle session management.

## What Happens When You Click It

**First Click (Activate):**
- Button changes from "Single" to "Sessions"
- Button styling becomes filled/prominent
- Banner appears: "🤖 Multi-Agent Mode Active"
- Welcome screen shows if no sessions exist

**Second Click (Deactivate):**
- Returns to single-agent mode
- Button shows "Single" again
- Session management UI disappears

## Troubleshooting

### "I Don't See the Button"
1. **Make sure Agent Panel is open** - Press `Cmd+?`
2. **Look carefully** - It's a medium-sized 👥 icon with text
3. **Check the right location** - Between [+] and [⋮] in toolbar
4. **Try keyboard shortcut** - `Cmd+Alt+M`
5. **Restart Zed** if needed

### "The Button Looks the Same"
- The button IS there, but styling might be subtle when OFF
- Look for the text label "Single" vs "Sessions"
- Try clicking it and watch for the banner to appear
- Use the keyboard shortcut to be sure

### "Nothing Happens When I Click"
- Make sure you're clicking the right button (👥 with text)
- Look for the "🤖 Multi-Agent Mode Active" banner
- Check if session tabs appear below the toolbar
- The change might be subtle at first

## Success Indicators

You'll know session management is working when you see:
1. Button text changes to "Sessions"
2. Button styling becomes more prominent
3. "🤖 Multi-Agent Mode Active" banner appears
4. Session creation interface becomes available
5. Welcome screen for creating first session

The session management feature transforms the single-agent chat into a multi-agent coordination system while keeping the same familiar interface.
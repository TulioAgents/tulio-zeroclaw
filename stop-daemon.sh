#!/bin/zsh
# Stop the zeroclaw launchd service (prevents auto-restart), then kill any remaining process
launchctl unload ~/Library/LaunchAgents/com.zeroclaw.daemon.plist 2>/dev/null && echo "Service stopped."

# Kill any leftover process still holding the port
if lsof -ti :42617 &>/dev/null; then
    kill $(lsof -ti :42617) 2>/dev/null && echo "Port 42617 released."
fi

echo "Done."

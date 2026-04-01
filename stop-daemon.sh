#!/bin/zsh
pkill -f "zeroclaw daemon" 2>/dev/null && echo "Daemon stopped." || echo "No daemon running."

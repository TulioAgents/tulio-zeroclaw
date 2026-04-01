#!/bin/zsh
# Kill any existing daemon before starting a new one
pkill -f "zeroclaw daemon" 2>/dev/null
sleep 1

source ~/.zshrc
cd /Users/javierhbr/agents/tulio-zeroclaw
ZEROCLAW_API_KEY=$MINIMAX_API_KEY ./target/debug/zeroclaw daemon "$@"

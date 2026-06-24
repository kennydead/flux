#!/bin/bash
# Claude Agent Farm — one-time Claude authentication

echo ""
echo "=== Claude Authentication ==="
echo ""
echo "This will open a shell inside the agent container."
echo "Once inside, run this command:"
echo ""
echo "    claude auth login"
echo ""
echo "A URL will appear — open it in your browser and log in with your claude.ai account."
echo "When done, type: exit"
echo ""
read -rp "Press Enter to continue..."

docker run --rm -it \
  -v claudeagentfarm_claude-home:/home/agent \
  ghcr.io/kennydead/claude-agent-farm/agent:latest \
  bash

echo ""
echo "Done! You can now spawn agents from the dashboard."

#!/bin/bash
# Claude Agent Farm — one-time Claude authentication

echo ""
echo "=== Claude Authentication ==="
echo ""
echo "A login URL will appear below."
echo "Copy it into your browser, log in with your claude.ai account,"
echo "then paste the code back here when prompted."
echo ""
echo "If it completes immediately, you are already authenticated."
echo ""

docker run --rm -i --platform linux/amd64 --entrypoint="" \
  -v agentfarm_claude-home:/home/agent \
  ghcr.io/kennydead/claude-agent-farm/agent:latest \
  claude auth login

echo ""
echo "Done! You can now spawn agents from the dashboard."

#!/bin/bash
# Claude Agent Farm — start script
set -e

mkdir -p logs
touch .env
touch config.yml

echo "Pulling latest images..."
docker pull ghcr.io/kennydead/claude-agent-farm/agent:latest
docker pull ghcr.io/kennydead/claude-agent-farm/dashboard-backend:latest
docker pull ghcr.io/kennydead/claude-agent-farm/dashboard-frontend:latest

echo "Tagging images..."
docker tag ghcr.io/kennydead/claude-agent-farm/agent:latest claudeagentfarm-coder
docker tag ghcr.io/kennydead/claude-agent-farm/agent:latest claudeagentfarm-reviewer
docker tag ghcr.io/kennydead/claude-agent-farm/agent:latest claudeagentfarm-planner
docker tag ghcr.io/kennydead/claude-agent-farm/agent:latest claudeagentfarm-auditor

docker compose up -d dashboard-db dashboard-backend dashboard-frontend

if command -v python3 >/dev/null 2>&1; then
    pkill -f "python3 host_bridge.py" 2>/dev/null || true
    nohup python3 host_bridge.py > logs/host_bridge.log 2>&1 &
fi

echo ""
echo "Dashboard running at http://localhost:5174"
echo ""

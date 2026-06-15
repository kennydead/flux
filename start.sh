#!/bin/bash
# Claude Agent Farm — start script
mkdir -p logs
touch .env
docker compose up -d dashboard-db dashboard-backend dashboard-frontend
echo ""
echo "Dashboard running at http://localhost:5174"
echo ""

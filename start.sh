#!/bin/bash
# Claude Agent Farm — start script
set -e

mkdir -p logs
touch .env
touch config.yml

# The farm's images are private: log docker in to ghcr.io with the read-only login the license server
# hands a valid license (owner 2026-11-06). Key from LICENSE_KEY or logs/license_key.txt (Flux saves it
# there). No key / no login → the pulls below stay anonymous. The token is never printed.
LICENSE_ENDPOINT="https://oywppqdqfcypawrthfox.supabase.co/functions/v1/validate-license"
KEY="${LICENSE_KEY:-$(cat logs/license_key.txt 2>/dev/null || true)}"
KEY="$(printf %s "$KEY" | tr -d '[:space:]')"
if [ -n "$KEY" ] && { ! command -v curl >/dev/null 2>&1 || ! command -v python3 >/dev/null 2>&1; }; then
    echo "curl and python3 are needed to check the license key — pulling without a login."
elif [ -n "$KEY" ]; then
    # The body is built by python (a key with quotes stays valid JSON) and sent on stdin (not in curl's argv).
    # The answer becomes "valid [user token]" or nothing.
    ANSWER="$(KEY="$KEY" python3 -c 'import json,os; print(json.dumps({"key": os.environ["KEY"]}))' \
        | curl -fsS -m 10 -X POST -H 'Content-Type: application/json' --data @- "$LICENSE_ENDPOINT" 2>/dev/null \
        | python3 -c 'import json,sys
d=json.load(sys.stdin); r=d.get("registry") or {}
if d.get("valid") is True:
    print("valid " + r["username"] + " " + r["token"] if r.get("username") and r.get("token") else "valid")' 2>/dev/null || true)"
    if [ -n "$ANSWER" ] && [ -n "${LICENSE_KEY:-}" ]; then
        # The farm itself reads the key from this file: save the one that just worked.
        ( umask 077; printf %s "$KEY" > logs/license_key.txt ) 2>/dev/null \
            || echo "Could not save the license key to logs/license_key.txt (check the file's owner)."
    fi
    LOGIN="${ANSWER#valid}"; LOGIN="${LOGIN# }"
    if [ -n "$LOGIN" ]; then
        printf %s "${LOGIN#* }" | docker login ghcr.io -u "${LOGIN%% *}" --password-stdin >/dev/null \
            && echo "Logged in to the farm's image registry." \
            || echo "Could not log in to the image registry — trying without a login."
    elif [ -z "$ANSWER" ]; then
        echo "The license key could not be confirmed right now — pulling without a new login."
    fi
    unset ANSWER LOGIN
else
    echo "No license key found — pulling without a login. Set it with: LICENSE_KEY=<your key> ./start.sh"
fi

# A pull refused because of an old saved login (ghcr.io refuses a stale one even for public images):
# retry with an empty docker config, on the same docker endpoint.
pull_anonymously() {
    local tmp host rc
    tmp="$(mktemp -d)"
    host="$(docker context inspect --format '{{.Endpoints.docker.Host}}' 2>/dev/null || true)"
    if [ -n "$host" ]; then
        DOCKER_CONFIG="$tmp" DOCKER_HOST="$host" docker pull "$1"; rc=$?
    else
        DOCKER_CONFIG="$tmp" docker pull "$1"; rc=$?
    fi
    rm -rf "$tmp"
    return $rc
}

echo "Pulling latest images..."
for img in agent dashboard-backend dashboard-frontend; do
    ref="ghcr.io/kennydead/claude-agent-farm/$img:latest"
    docker pull "$ref" || pull_anonymously "$ref" || {
        echo "Could not download the farm's images (see the message above)."
        echo "Check the internet connection and that Docker is running. The images need a valid license key:"
        echo "  LICENSE_KEY=<your key> ./start.sh"
        exit 1
    }
done

echo "Tagging images..."
docker tag ghcr.io/kennydead/claude-agent-farm/agent:latest agentfarm-coder
docker tag ghcr.io/kennydead/claude-agent-farm/agent:latest agentfarm-reviewer
docker tag ghcr.io/kennydead/claude-agent-farm/agent:latest agentfarm-planner
docker tag ghcr.io/kennydead/claude-agent-farm/agent:latest agentfarm-auditor

docker compose up -d dashboard-db dashboard-backend dashboard-frontend

if command -v python3 >/dev/null 2>&1; then
    pkill -f "python3 host_bridge.py" 2>/dev/null || true
    nohup python3 host_bridge.py > logs/host_bridge.log 2>&1 &
fi

echo ""
echo "Dashboard running at http://localhost:5174"
echo ""

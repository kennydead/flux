"""
Host bridge — runs natively on the host (not in Docker).
Listens on localhost:8092 and opens a terminal for interactive planning sessions.
"""
import json
import os
import platform
import subprocess
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = 8092
PROJECT_DIR = os.path.dirname(os.path.abspath(__file__))
SYSTEM = platform.system()
IMAGE = "ghcr.io/kennydead/claude-agent-farm/agent:latest"

try:
    with open("/proc/version") as f:
        IS_WSL = "microsoft" in f.read().lower()
except OSError:
    IS_WSL = False


def _open_terminal(project_dir: str, project_id: str) -> None:
    workspace = os.path.join(project_dir, "planning-workspace")
    os.makedirs(workspace, exist_ok=True)

    env_flags = "-e DASHBOARD_URL=http://dashboard-backend:8090"
    if project_id:
        env_flags += f" -e DASHBOARD_PROJECT_ID={project_id}"

    docker_cmd = (
        f"docker run -it --rm --platform linux/amd64 "
        f"--network claudeagentfarm_default "
        f"{env_flags} "
        f"-v \"{workspace}\":/planning-workspace "
        f"{IMAGE} python -m agent.discuss"
    )

    if SYSTEM == "Darwin":
        safe = docker_cmd.replace("\\", "\\\\").replace('"', '\\"')
        script = (
            f'tell application "Terminal" to activate\n'
            f'tell application "Terminal" to do script "{safe}"'
        )
        subprocess.Popen(["osascript", "-e", script])

    elif IS_WSL:
        try:
            subprocess.Popen(
                ["wt.exe", "-w", "0", "nt", "--", "bash", "-c", f"{docker_cmd}; exec bash"]
            )
        except FileNotFoundError:
            subprocess.Popen(
                ["cmd.exe", "/c", f'start "Farm Discuss" bash -c "{docker_cmd}"']
            )

    elif SYSTEM == "Windows":
        subprocess.Popen(f'start "Farm Discuss" cmd /k "{docker_cmd}"', shell=True)

    else:
        display = os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY")
        if not display:
            print("Host bridge: no display detected — cannot open terminal")
            return
        for term in [
            ["gnome-terminal", "--", "bash", "-c", f"{docker_cmd}; exec bash"],
            ["xterm", "-e", f"bash -c '{docker_cmd}; exec bash'"],
        ]:
            try:
                subprocess.Popen(term)
                return
            except FileNotFoundError:
                continue


class Handler(BaseHTTPRequestHandler):
    def _cors(self):
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")

    def do_OPTIONS(self):
        self.send_response(204)
        self._cors()
        self.end_headers()

    def do_POST(self):
        if self.path == "/open-discuss":
            length = int(self.headers.get("Content-Length", 0))
            body = {}
            if length:
                try:
                    body = json.loads(self.rfile.read(length))
                except Exception:
                    pass
            _open_terminal(PROJECT_DIR, body.get("projectId", ""))
            self.send_response(200)
            self._cors()
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"ok":true}')
        else:
            self.send_response(404)
            self._cors()
            self.end_headers()

    def log_message(self, *_):
        pass


if __name__ == "__main__":
    print(f"Host bridge listening on http://localhost:{PORT} ({SYSTEM}{'[WSL]' if IS_WSL else ''})")
    HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()

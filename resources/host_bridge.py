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

    # claude-home volume carries the Claude Code auth credentials
    docker_args = [
        "docker", "run", "-it", "--rm", "--platform", "linux/amd64",
        "--network", "claudeagentfarm_default",
        "-e", "DASHBOARD_URL=http://dashboard-backend:8090",
        "-v", "claudeagentfarm_claude-home:/home/agent",
        "-v", f"{workspace}:/planning-workspace",
        IMAGE, "python", "-m", "agent.discuss",
    ]
    if project_id:
        docker_args = docker_args[:6] + ["-e", f"DASHBOARD_PROJECT_ID={project_id}"] + docker_args[6:]

    # Plain string form for shells that need it (Mac/Linux)
    def _shell_str():
        return " ".join(
            f'"{a}"' if (" " in a or ":" in a) else a
            for a in docker_args
        )

    if SYSTEM == "Darwin":
        cmd = _shell_str()
        safe = cmd.replace("\\", "\\\\").replace('"', '\\"')
        script = (
            f'tell application "Terminal" to activate\n'
            f'tell application "Terminal" to do script "{safe}"'
        )
        subprocess.Popen(["osascript", "-e", script])

    elif IS_WSL:
        # Pass args as a list to wt.exe to avoid Windows quoting issues
        try:
            subprocess.Popen(["wt.exe", "-w", "0", "nt", "--"] + docker_args)
        except FileNotFoundError:
            subprocess.Popen(["cmd.exe", "/c", "start", "Farm Discuss"] + docker_args)

    elif SYSTEM == "Windows":
        # Write a temp .bat file to avoid embedded-quote issues in cmd /k "..."
        import tempfile
        bat = tempfile.NamedTemporaryFile(
            mode="w", suffix=".bat", delete=False, dir=project_dir
        )
        bat.write("@echo off\n" + " ".join(docker_args) + "\npause\n")
        bat.close()
        subprocess.Popen(
            ["cmd.exe", "/c", "start", "Farm Discuss", "cmd", "/k", bat.name]
        )

    else:
        display = os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY")
        if not display:
            print("Host bridge: no display detected — cannot open terminal")
            return
        cmd = _shell_str()
        for term in [
            ["gnome-terminal", "--", "bash", "-c", f"{cmd}; exec bash"],
            ["xterm", "-e", f"bash -c '{cmd}; exec bash'"],
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

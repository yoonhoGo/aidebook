#!/usr/bin/env python3
"""Exercise actual CLI/MCP and a disposable Core owner; never use the user's DB."""
import json
import pathlib
import subprocess
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = ROOT / "src-tauri/target/debug"


def run():
    with tempfile.TemporaryDirectory(prefix="ab-workflow-", dir="/tmp") as directory:
        data = pathlib.Path(directory)
        owner = None

        def call(action, payload):
            result = subprocess.run(
                [str(BIN / "aidebook-cli"), "workflow", action, "--data-dir", str(data), "--params", json.dumps(payload)],
                text=True, capture_output=True, timeout=10,
            )
            if result.returncode:
                raise RuntimeError(result.stderr)
            return json.loads(result.stdout)

        def start():
            process = subprocess.Popen([
                str(BIN / "aidebook-core"), "--db", str(data / "core.sqlite"),
                "--socket", str(data / "aidebook-core.sock"),
                "--token-file", str(data / "aidebook-core.token"),
            ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            for _ in range(100):
                try:
                    call("list", {"kind": "work"})
                    return process
                except (RuntimeError, FileNotFoundError):
                    if process.poll() is not None:
                        raise RuntimeError("fixture Core owner exited")
                    time.sleep(0.05)
            process.terminate()
            process.wait(timeout=5)
            raise RuntimeError("fixture Core owner did not become ready")

        try:
            owner = start()
            request = {"kind": "work", "idempotency_key": "smoke-work", "fields": {"title": "CLI 실제 업무", "status": "planned"}}
            work = call("save", request)["item"]
            replay = call("save", request)
            assert replay["item"] == work and replay["idempotent_replay"]
            task = call("save", {"kind": "task", "idempotency_key": "smoke-task", "fields": {
                "title": "작업 시간 보존", "status": "in_progress", "work_id": work["id"],
                "target_date": "2026-09-25", "time_blocks": [{"start": "2026-09-25T10:00:00+09:00", "end": "2026-09-25T11:00:00+09:00"}],
            }})["item"]
            owner.terminate()
            owner.wait(timeout=5)
            owner = start()
            assert call("get", {"kind": "task", "id": task["id"]}) == task
            messages = [
                {"jsonrpc": "2.0", "id": 1, "method": "tools/list"},
                {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "workflow.list", "arguments": {"kind": "work"}}},
            ]
            mcp = subprocess.run([str(BIN / "aidebook-cli"), "mcp", "serve", "--stdio", "--data-dir", str(data)],
                                 input="\n".join(map(json.dumps, messages)) + "\n", text=True, capture_output=True, timeout=10, check=True)
            replies = [json.loads(line) for line in mcp.stdout.splitlines()]
            assert any(tool["name"] == "workflow.save" for tool in replies[0]["result"]["tools"])
            content = replies[1]["result"]["content"][0]["text"]
            assert json.loads(content)[0]["id"] == work["id"]
            print(json.dumps({"cli_create_replay": "passed", "owner_restart_preserves_task_and_blocks": "passed", "mcp_tools_and_shared_data": "passed", "real_accounts_used": False}, ensure_ascii=False, indent=2))
        finally:
            if owner is not None and owner.poll() is None:
                owner.terminate()
                owner.wait(timeout=5)


if __name__ == "__main__":
    run()

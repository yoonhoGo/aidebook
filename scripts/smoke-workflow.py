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

        def api(group, action, payload):
            result = subprocess.run(
                [str(BIN / "aidebook-cli"), group, action, "--data-dir", str(data), "--params", json.dumps(payload)],
                text=True, capture_output=True, timeout=10,
            )
            if result.returncode:
                raise RuntimeError(result.stderr)
            return json.loads(result.stdout)

        def call(action, payload):
            return api("workflow", action, payload)

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
            source = json.loads((ROOT / "src-tauri/fixtures/github.json").read_text())["snapshots"][0]["source"]
            api("sources", "refresh", {"fixture_path": str(ROOT / "src-tauri/fixtures/github.json")})
            link = api("work-link", "add", {"work_id": work["id"], "target_kind": "source", "target_source": source,
                "relation_type": "context", "reason": "explicit fixture link", "idempotency_key": "smoke-link"})["link"]
            assert link["access_status"] == "accessible"
            api("work-link", "remove", {"id": link["id"], "expected_version": link["version"], "idempotency_key": "smoke-unlink"})
            assert api("work-link", "list", {"work_id": work["id"]}) == []
            legacy = {"namespace": "smoke-browser", "groups": [{"legacy_work_index": 0, "title": "명시 가져오기", "unmigrated_note_count": 1}]}
            assert call("import-preview", legacy)["groups"][0]["skipped"]
            imported = call("import-apply", {**legacy, "idempotency_key": "smoke-import"})
            assert call("import-apply", {**legacy, "idempotency_key": "smoke-import"})["works"] == imported["works"]
            dashboard = api("dashboard", "get", {"timezone": "Asia/Seoul", "section": "next", "now": "2026-09-23T10:00:00Z"})
            assert any(entry["item"]["id"] == task["id"] for entry in dashboard["entries"])
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
            assert any(item["id"] == work["id"] for item in json.loads(content))
            print(json.dumps({"cli_create_replay": "passed", "owner_restart_preserves_task_and_blocks": "passed", "mcp_tools_and_shared_data": "passed", "explicit_link_add_remove": "passed", "legacy_preview_apply_replay": "passed", "dashboard_shared_tasks": "passed", "real_accounts_used": False}, ensure_ascii=False, indent=2))
        finally:
            if owner is not None and owner.poll() is None:
                owner.terminate()
                owner.wait(timeout=5)


if __name__ == "__main__":
    run()

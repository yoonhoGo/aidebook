#!/usr/bin/env python3
"""Test a running isolated native host and restart its exact executable (not UI QA).
First launch the probe using the config/command in WORKFLOW_NATIVE_VALIDATION.md.
Never point this harness at a normal Aidebook process or user data directory.
"""
import argparse, json, os, pathlib, signal, subprocess, time, shutil, tempfile
root = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--pid", required=True, type=int)
args = parser.parse_args()
data = pathlib.Path.home() / "Library/Application Support/dev.aidebook.wp923"
binary = root / "src-tauri/target/debug/aidebook"
cli = root / "src-tauri/target/debug/aidebook-cli"
# Require evidence that this PID owns the dedicated probe DB, not another app.
files = subprocess.check_output(["lsof", "-p", str(args.pid)], text=True)
assert str(data / "aidebook.sqlite") in files, "PID does not own the isolated probe DB"
def call(action, payload):
    result = subprocess.run([str(cli), "workflow", action, "--data-dir", str(data), "--params", json.dumps(payload)], capture_output=True, text=True, timeout=10, check=True)
    return json.loads(result.stdout)
work = call("save", {"kind":"work", "idempotency_key":"native-probe-work", "fields":{"title":"Native host IPC probe", "status":"planned"}})["item"]
task = call("save", {"kind":"task", "idempotency_key":"native-probe-task", "fields":{"title":"Native restart task", "status":"planned", "work_id":work["id"], "time_blocks":[{"start":"2026-09-25T10:00:00+09:00", "end":"2026-09-25T11:00:00+09:00"}]}})["item"]
# Copy before terminating: concurrent builds must not replace our restart target.
probe_binary_dir = tempfile.TemporaryDirectory(prefix="ab-native-restart-")
restart_binary = pathlib.Path(probe_binary_dir.name) / "aidebook"
shutil.copy2(binary, restart_binary)
assert b'dev.aidebook.wp923' in restart_binary.read_bytes(), "binary lacks isolated probe config"
os.kill(args.pid, signal.SIGTERM)
time.sleep(1)
# A stopped app cannot serve IPC. CLI must not silently become a writer.
stopped = subprocess.run([str(cli), "workflow", "list", "--data-dir", str(data), "--params", '{"kind":"work"}'], capture_output=True, text=True)
assert stopped.returncode != 0
process = subprocess.Popen([str(restart_binary)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    for attempt in range(100):
        try:
            restored = call("get", {"kind":"task", "id":task["id"]})
            break
        except subprocess.CalledProcessError:
            if process.poll() is not None: raise RuntimeError("native restart exited")
            time.sleep(.1)
    else: raise RuntimeError("native IPC restart timed out")
    assert restored == task
    print(json.dumps({"host":"real Tauri debug process", "isolated_identifier":"dev.aidebook.wp923", "ipc_create":"passed", "process_termination_removes_ipc_service":"passed", "native_host_restart_preserves_task_and_blocks":"passed", "ui_save_verified":False, "window_close_verified":False, "real_accounts_used":False}, indent=2))
finally:
    process.terminate()
    process.wait(timeout=10)
    probe_binary_dir.cleanup()

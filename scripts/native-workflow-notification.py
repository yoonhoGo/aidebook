#!/usr/bin/env python3
"""Compile/run a distinct temporary app; read notification permission without prompting."""
import json, pathlib, plistlib, subprocess, tempfile
root = pathlib.Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="aidebook-notification-probe-") as temporary:
    app = pathlib.Path(temporary) / "AidebookNotificationProbe.app" / "Contents"
    binary = app / "MacOS" / "probe"
    binary.parent.mkdir(parents=True)
    (app / "Info.plist").write_bytes(plistlib.dumps({
        "CFBundleIdentifier": "com.yoonhogo.aidebook.notification-probe",
        "CFBundleExecutable": "probe", "CFBundleName": "Aidebook Notification Probe",
        "CFBundlePackageType": "APPL", "LSUIElement": True,
    }))
    subprocess.run(["xcrun", "swiftc", str(root / "scripts/native-workflow-notification.swift"), "-o", str(binary)], check=True)
    subprocess.run(["codesign", "--force", "--sign", "-", str(app.parent)], check=True, capture_output=True)
    result = subprocess.run([str(binary)], text=True, capture_output=True, timeout=20)
    print(result.stdout, end="")
    if result.returncode:
        raise RuntimeError(result.stderr)

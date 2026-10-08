#!/usr/bin/env python3
"""Run the real local API in demo mode; never contacts or changes a router."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import urllib.error
import urllib.request

root = Path(__file__).resolve().parent
with tempfile.TemporaryDirectory(prefix="web-check-", dir=root / "target") as tmp:
    config = Path(tmp) / "settings.json"
    config.write_text((root / "settings.example.json").read_text())
    process = subprocess.Popen(
        [str(root / "dist/MikroCockpit.app/Contents/MacOS/MikroCockpit"), "--demo", "--print-url"],
        env={**os.environ, "MIKROCOCKPIT_CONFIG": str(config)}, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, text=True,
    )
    try:
        url = process.stdout.readline().strip()
        assert url.startswith("http://127.0.0.1:"), url
        base, token = url.split("/#")
        def request(path, auth=None, data=None, origin=None, host=None):
            headers = {"Content-Type": "application/json"}
            if auth: headers["X-MikroCockpit-Token"] = auth
            if origin: headers["Origin"] = origin
            if host: headers["Host"] = host
            req = urllib.request.Request(base + path, headers=headers,
                data=json.dumps(data).encode() if data is not None else None)
            try:
                with urllib.request.urlopen(req, timeout=5) as response:
                    return response.status, response.read().decode(), response.headers
            except urllib.error.HTTPError as response:
                return response.code, response.read().decode(), response.headers
        for path in ["/", "/app.js", "/style.css"]:
            status, body, headers = request(path)
            assert status == 200 and body
            assert headers["Content-Security-Policy"] and headers["Cache-Control"] == "no-store"
        with urllib.request.urlopen(base + "/logo.png", timeout=5) as response:
            assert response.headers["Content-Type"] == "image/png"
            assert response.read() == (root / "assets/logo.png").read_bytes()
        assert request("/logo.png", host="attacker.test")[0] == 403
        assert request("/api/state")[0] == 403
        assert request("/api/state", "wrong")[0] == 403
        assert request("/api/state", token, host="attacker.test")[0] == 403
        assert request("/api/action", token, {"kind": "refresh"}, "https://attacker.test")[0] == 403
        assert request("/api/action", token, {"kind": "command", "script": "/system reboot"}, base)[0] == 400
        assert request("/api/action", token, {"kind": "direct", "address": "all"}, base)[0] == 400
        assert request("/api/action", token, {"kind": "refresh"}, base)[0] == 202
        prefs = {"theme": "light", "icon_color": "purple", "hidden": ["wireguard"], "tray_order": ["routes", "direct"], "tray_title": "custom", "status_fields": ["connection", "speed", "interfaces"], "status_interfaces": ["ether1"]}
        assert request("/api/preferences", token, prefs, base)[0] == 202
        status, body, _ = request("/api/state", token)
        assert status == 200 and json.loads(body)["preferences"] == prefs
        assert json.loads(body)["locale"]["language"] == "en"
        assert isinstance(json.loads(body)["locale"]["preferred"], list)
        assert 'lang="en"' in request("/")[1]
        assert "MikroCockpit" in request("/")[1]
        assert json.loads((Path(tmp) / "state/preferences.json").read_text()) == prefs
        print("HTTP assets, token, Host/Origin, typed actions and persistent preferences: OK")
    finally:
        process.terminate()
        process.wait(timeout=5)

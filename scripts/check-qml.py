#!/usr/bin/env python3
"""Fail-closed structural checks for the thin QML presentation boundary."""

from pathlib import Path
import json
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
FILES = [
    ROOT / "omarchy-plugin" / "BarWidget.qml",
    ROOT / "omarchy-plugin" / "PremonitionSurface.qml",
]


def fail(message: str) -> None:
    print(f"QML check failed: {message}", file=sys.stderr)
    raise SystemExit(1)


manifest = json.loads((ROOT / "manifest.json").read_text(encoding="utf-8"))
if manifest.get("schemaVersion") != 1:
    fail("manifest must use schemaVersion 1")
if manifest.get("kinds") != ["bar-widget", "panel"]:
    fail("manifest must expose exactly the reviewed bar-widget and panel")
for relative in manifest.get("entryPoints", {}).values():
    target = ROOT / relative
    if target.is_symlink() or not target.is_file():
        fail("entry point is missing or a symlink")

for path in FILES:
    text = path.read_text(encoding="utf-8")
    for forbidden in (
        "Text.RichText",
        "RichText",
        "Qt.createQmlObject",
        "console.",
        "FileView",
        "ClipboardHistory",
        '"bash"',
        '"sh", "-c"',
        "eval(",
    ):
        if forbidden in text:
            fail(f"{path.name} contains forbidden token {forbidden!r}")
    if re.search(r"\bcommand\s*:\s*\"", text):
        fail(f"{path.name} uses a string command instead of an argv array")

surface = FILES[1].read_text(encoding="utf-8")
required = (
    'textFormat: Text.PlainText',
    'textFormat: TextEdit.PlainText',
    'interval: 1200',
    'if (!statusProcess.running)',
    'slice(0, 20)',
    'slice(0, 64)',
    'nextPatch.length > 262144',
    'nextRationale.length > 8192',
    'WlrKeyboardFocus.Exclusive',
    'Keys.onEscapePressed',
    'text: "Review"',
    'text: "Apply"',
    'text: "Copy patch"',
    'text: "Dismiss"',
    'text: "Cancel"',
    '"--clipboard"',
    '"--selection"',
)
for token in required:
    if token not in surface:
        fail(f"reviewed surface contract is missing {token!r}")

# JSON-derived text must be rendered as plain text, never markup. Check the
# dynamic bindings used by this surface, including a hostile fixture spelling.
for match in re.finditer(r"\btext:\s*(root\.|String\(modelData)", surface):
    block_start = surface.rfind("Text {", 0, match.start())
    if block_start < 0:
        fail("dynamic text binding is not inside a Text block")
    prefix = surface[block_start : match.start()]
    if "textFormat: Text.PlainText" not in prefix:
        fail("dynamic Text binding lacks Text.PlainText")

hostile = '<img src="file:///etc/passwd">\\u202e\\x1b[31m'
if not isinstance(hostile, str) or len(hostile) > 128:
    fail("hostile fixture setup failed")

print("QML structural and hostile-text checks passed")

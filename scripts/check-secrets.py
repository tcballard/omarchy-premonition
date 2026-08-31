#!/usr/bin/env python3
"""Small deterministic credential-pattern check for repository inputs."""

from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
MAX_FILE_BYTES = 2 * 1024 * 1024
PATTERNS = {
    "private key": re.compile(
        rb"-----" + rb"BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"
    ),
    "GitHub token": re.compile(rb"gh[opsu]_[A-Za-z0-9]{30,}"),
    "OpenAI token": re.compile(rb"sk-[A-Za-z0-9_-]{40,}"),
    "AWS access key": re.compile(rb"AKIA[0-9A-Z]{16}"),
}


def repository_files() -> list[Path]:
    output = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    return [ROOT / value.decode("utf-8") for value in output.split(b"\0") if value]


findings: list[str] = []
for path in repository_files():
    if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_FILE_BYTES:
        continue
    data = path.read_bytes()
    for name, pattern in PATTERNS.items():
        if pattern.search(data):
            findings.append(f"{path.relative_to(ROOT)}: possible {name}")

if findings:
    print("Credential-pattern check failed:", file=sys.stderr)
    print("\n".join(findings), file=sys.stderr)
    raise SystemExit(1)

print("Credential-pattern check passed")

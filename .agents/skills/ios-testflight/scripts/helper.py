#!/usr/bin/env python3
"""Install or invoke the signed TestFlight operation helper in the user's GUI session."""

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import uuid

IDENTIFIER = "com.naituw.askhuman.testflight-helper"
IDENTITY = "Developer ID Application: Wu Tian (DMJXDB9H6Q)"


def interrupted(*_args):
    raise KeyboardInterrupt


def verify(binary):
    subprocess.run(["/usr/bin/codesign", "--verify", "--strict", str(binary)], check=True, capture_output=True)
    result = subprocess.run(["/usr/bin/codesign", "-dv", "--verbose=4", str(binary)], check=True, capture_output=True, text=True)
    if f"Identifier={IDENTIFIER}\n" not in result.stderr or "TeamIdentifier=DMJXDB9H6Q\n" not in result.stderr:
        raise RuntimeError("The helper must have its established identifier and Developer ID team signature.")


def install(root):
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    if root.stat().st_mode & 0o077:
        raise RuntimeError("The local configuration directory must be owner-only.")
    with tempfile.TemporaryDirectory(prefix="helper-build-", dir=root) as folder:
        binary = Path(folder) / "testflight-helper"
        source = Path(__file__).with_name("TestFlightHelper.swift")
        subprocess.run(["xcrun", "swiftc", "-parse-as-library", "-O", str(source), "-o", str(binary)], check=True)
        subprocess.run(["/usr/bin/codesign", "--force", "--timestamp=none", "-i", IDENTIFIER, "--sign", IDENTITY, str(binary)], check=True)
        verify(binary)
        binary.chmod(0o700)
        os.replace(binary, root / "testflight-helper")
    print("Installed the stable signed TestFlight helper; no credentials were displayed.")


def invoke(action, archive, config):
    import plistlib

    binary = config.parent / "testflight-helper"
    verify(binary)
    domain = f"gui/{os.getuid()}"
    label = "com.naituw.askhuman.testflight." + uuid.uuid4().hex
    target = domain + "/" + label
    with tempfile.TemporaryDirectory(prefix="testflight-operation-") as folder:
        folder = Path(folder)
        result = folder / "result.json"
        stdout, stderr = folder / "stdout", folder / "stderr"
        for path in (stdout, stderr):
            path.touch(mode=0o600)
        args = [str(binary), "--config", str(config), "--result", str(result), action]
        if archive is not None:
            args.append(str(archive.resolve()))
        plist = folder / "operation.plist"
        plist.write_bytes(plistlib.dumps({
            "Label": label, "ProgramArguments": args, "RunAtLoad": True,
            "StandardOutPath": str(stdout), "StandardErrorPath": str(stderr),
            "ProcessType": "Background",
        }))
        loaded = False
        try:
            # A cancelled bootstrap can still have registered the one-shot job.
            loaded = True
            subprocess.run(["launchctl", "bootstrap", domain, str(plist)], check=True, capture_output=True)
            deadline = time.monotonic() + (1860 if action in ("upload", "export") else 90)
            while not result.exists():
                if time.monotonic() > deadline:
                    raise RuntimeError("The helper did not finish. Check Keychain authorization through the user.")
                time.sleep(0.2)
            return json.loads(result.read_text())["exitCode"]
        finally:
            if loaded:
                present = subprocess.run(["launchctl", "print", target], capture_output=True).returncode == 0
                if present and not result.exists():
                    subprocess.run(["launchctl", "kill", "SIGTERM", target], capture_output=True)
                    deadline = time.monotonic() + 20
                    while not result.exists() and time.monotonic() < deadline:
                        time.sleep(0.2)
                subprocess.run(["launchctl", "bootout", target], capture_output=True)
            sys.stdout.write(stdout.read_text())
            sys.stderr.write(stderr.read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["install", "migrate", "status", "export", "upload"])
    parser.add_argument("archive", nargs="?", type=Path)
    parser.add_argument("--config", type=Path, default=Path.home() / ".config/askhuman-release/app-store-connect.json")
    args = parser.parse_args()
    config = args.config.expanduser().resolve()
    if args.action == "install":
        install(config.parent)
        return 0
    if (args.action in ("upload", "export")) != (args.archive is not None):
        parser.error("Only upload/export require an archive path.")
    return invoke(args.action, args.archive, config)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("TestFlight operation interrupted; check Apple status before another upload.", file=sys.stderr)
        sys.exit(130)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"TestFlight helper: {error}", file=sys.stderr)
        sys.exit(1)

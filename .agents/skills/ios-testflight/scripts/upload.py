#!/usr/bin/env python3
"""Inspect an archive; use the signed Keychain helper for explicit upload/signing checks."""

import argparse
import json
import os
from pathlib import Path
import plistlib
import signal
import subprocess
import sys

sys.dont_write_bytecode = True
from helper import interrupted, invoke, verify

BUNDLE_ID = "com.naituw.humaninloop.ios"
TEAM_ID = "DMJXDB9H6Q"


def prepare(archive, config_path):
    root = config_path.parent
    if root.stat().st_uid != os.getuid() or root.stat().st_mode & 0o077:
        raise RuntimeError("The local configuration directory must be owner-only.")
    config = json.loads(config_path.read_text())
    if config.get("credentialBackend") != "keychain" or "privateKeyPath" in config:
        raise RuntimeError("Complete the approved Keychain migration; plaintext credentials are not supported.")
    verify(root / "testflight-helper")
    options = plistlib.loads((root / "testflight-export-options.plist").read_bytes())
    expected = {"method": "app-store-connect", "destination": "upload", "signingStyle": "manual",
                "teamID": TEAM_ID, "testFlightInternalTestingOnly": True}
    if any(options.get(key) != value for key, value in expected.items()):
        raise RuntimeError("Export options must use the established manual, internal-only distribution setup.")
    if not {BUNDLE_ID, BUNDLE_ID + ".notification"}.issubset(options.get("provisioningProfiles", {})):
        raise RuntimeError("Export options need profiles for both the app and its notification extension.")
    app = archive / "Products/Applications/AskHuman.app"
    info = plistlib.loads((app / "Info.plist").read_bytes())
    if info.get("CFBundleIdentifier") != BUNDLE_ID or info.get("UIDeviceFamily") != [1]:
        raise RuntimeError("Unexpected bundle ID or device family; regenerate the project and rebuild.")
    if info.get("AHPushEnvironment") != "prod" or info.get("ITSAppUsesNonExemptEncryption") is not False:
        raise RuntimeError("Review the production push environment and encryption declaration before distributing.")
    if not (app / "PrivacyInfo.xcprivacy").is_file():
        raise RuntimeError("The archive is missing its privacy manifest.")
    if not info.get("CFBundleShortVersionString") or not info.get("CFBundleVersion"):
        raise RuntimeError("The archive is missing its version or build number.")
    extension = plistlib.loads((app / "PlugIns/AskHumanNotification.appex/Info.plist").read_bytes())
    if extension.get("CFBundleIdentifier") != BUNDLE_ID + ".notification" or any(
        extension.get(key) != info[key] for key in ("CFBundleVersion", "CFBundleShortVersionString", "UIDeviceFamily")
    ):
        raise RuntimeError("The notification extension must match the app's version, build, and device family.")
    print(json.dumps({"archive": str(archive), "bundleId": BUNDLE_ID,
                      "version": info["CFBundleShortVersionString"], "build": info["CFBundleVersion"],
                      "credentialBackend": "keychain", "internalOnly": True}, indent=2), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--config", type=Path, default=Path.home() / ".config/askhuman-release/app-store-connect.json")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--upload", action="store_true", help="Deliver through internal TestFlight")
    mode.add_argument("--check-signing", action="store_true", help="Export locally without uploading to Apple")
    args = parser.parse_args()
    archive, config = args.archive.resolve(), args.config.expanduser().resolve()
    prepare(archive, config)
    if args.upload or args.check_signing:
        return invoke("upload" if args.upload else "export", archive, config)
    print("Preflight passed. Nothing uploaded; add --upload for an authorized delivery.")
    return 0


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("Operation interrupted; check Apple status before another upload.", file=sys.stderr)
        sys.exit(130)
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"TestFlight: {error}", file=sys.stderr)
        sys.exit(1)

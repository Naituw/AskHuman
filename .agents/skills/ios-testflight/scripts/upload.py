#!/usr/bin/env python3
"""Inspect an AskHuman archive; upload only with an explicit --upload flag."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import plistlib
import shlex
import signal
import subprocess
import sys

BUNDLE_ID = "com.naituw.humaninloop.ios"
TEAM_ID = "DMJXDB9H6Q"


def private_file(path):
    info = path.stat()
    if not path.is_file() or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise RuntimeError(f"Expected an owner-only credential file: {path}")


def prepare(archive, config_path):
    root = config_path.parent
    if root.stat().st_uid != os.getuid() or root.stat().st_mode & 0o077:
        raise RuntimeError("The credential directory must be owner-only.")
    config = json.loads(config_path.read_text())
    private_file(Path(config["privateKeyPath"]))
    private_file(root / "signing-keychain-password")
    keychain = root / "testflight-signing.keychain-db"
    if not keychain.is_file():
        raise RuntimeError("The configured local signing keychain is missing.")
    options_path = root / "testflight-export-options.plist"
    options = plistlib.loads(options_path.read_bytes())
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
    if info.get("AHPushEnvironment") != "prod":
        raise RuntimeError("TestFlight requires the app's production push environment.")
    if info.get("ITSAppUsesNonExemptEncryption") is not False:
        raise RuntimeError("Review the encryption declaration before distributing this archive.")
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
                      "deviceFamily": info["UIDeviceFamily"], "pushEnvironment": "prod",
                      "internalOnly": True}, indent=2), flush=True)
    return config, root, keychain, options_path


def upload(archive, config, root, keychain, options_path):
    with (root / "testflight-upload.lock").open("a") as lock:
        os.fchmod(lock.fileno(), 0o600)
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError("Another TestFlight upload is using this signing configuration; wait for it to finish.") from None
        return upload_locked(archive, config, root, keychain, options_path)


def upload_locked(archive, config, root, keychain, options_path):
    password = (root / "signing-keychain-password").read_text()

    def run(args):
        result = subprocess.run(args, capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError(result.stderr.replace(password, "<redacted>"))
        return result.stdout

    def search_list():
        return shlex.split(run(["security", "list-keychains", "-d", "user"]))

    def interrupted(_signum, _frame):
        raise KeyboardInterrupt

    before = search_list()
    keychain_path = str(keychain)
    added = keychain_path not in before
    child = None
    old_handler = signal.signal(signal.SIGTERM, interrupted)
    try:
        run(["security", "unlock-keychain", "-p", password, keychain_path])
        if added:
            run(["security", "list-keychains", "-d", "user", "-s", *before, keychain_path])
        child = subprocess.Popen([
            "xcodebuild", "-exportArchive", "-archivePath", str(archive),
            "-exportOptionsPlist", str(options_path), "-exportPath", str(archive.parent / "TestFlightExport"),
            "-authenticationKeyPath", config["privateKeyPath"], "-authenticationKeyID", config["keyId"],
            "-authenticationKeyIssuerID", config["issuerId"],
        ], start_new_session=True)
        return child.wait(timeout=1800)
    finally:
        if child is not None and child.poll() is None:
            os.killpg(child.pid, signal.SIGTERM)
            try:
                child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
        if added:
            # Remove only our addition, preserving other changes made during the upload.
            current = [item for item in search_list() if item != keychain_path]
            run(["security", "list-keychains", "-d", "user", "-s", *current])
        signal.signal(signal.SIGTERM, old_handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--config", type=Path, default=Path.home() / ".config/askhuman-release/app-store-connect.json")
    parser.add_argument("--upload", action="store_true", help="Upload the inspected archive to internal TestFlight")
    args = parser.parse_args()
    archive = args.archive.resolve()
    prepared = prepare(archive, args.config.expanduser().resolve())
    if args.upload:
        return upload(archive, *prepared)
    print("Preflight passed. Nothing uploaded; add --upload for an authorized delivery.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("Upload interrupted. Check Apple status before retrying.", file=sys.stderr)
        sys.exit(130)
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"TestFlight: {error}", file=sys.stderr)
        sys.exit(1)

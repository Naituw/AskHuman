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
# The project ships to iPhone (1) and iPad (2). TestFlightHelper.swift enforces the same pair.
DEVICE_FAMILY = [1, 2]
IPAD_ORIENTATIONS = {
    "UIInterfaceOrientationPortrait",
    "UIInterfaceOrientationPortraitUpsideDown",
    "UIInterfaceOrientationLandscapeLeft",
    "UIInterfaceOrientationLandscapeRight",
}
DEVICE_FAMILY_ADVICE = (
    "the project ships to iPhone and iPad (device family 1,2) for both the app and its extension. "
    "If that changed on purpose, update upload.py and TestFlightHelper.swift together and reinstall the helper; "
    "otherwise fix TARGETED_DEVICE_FAMILY, regenerate the project, and rebuild."
)


def has_device_family(info):
    """True when UIDeviceFamily is exactly the shipped pair of genuine integers, in any order."""
    families = info.get("UIDeviceFamily")
    return (isinstance(families, list) and all(type(item) is int for item in families)
            and sorted(families) == DEVICE_FAMILY)


def declares_ipad_orientations(info):
    """Apple rejects an iPad build that neither supports all four orientations nor opts out of multitasking."""
    if info.get("UIRequiresFullScreen") is True:
        return True
    # On iPad the suffixed key takes over completely, so the generic list is only a fallback.
    declared = info.get("UISupportedInterfaceOrientations~ipad", info.get("UISupportedInterfaceOrientations"))
    return isinstance(declared, list) and IPAD_ORIENTATIONS.issubset(item for item in declared if isinstance(item, str))


def inspect_archive(archive):
    """Check the archived app and its extension; return (version, build). Reads only files inside the archive."""
    app = archive / "Products/Applications/AskHuman.app"
    info = plistlib.loads((app / "Info.plist").read_bytes())
    if info.get("CFBundleIdentifier") != BUNDLE_ID:
        raise RuntimeError("Unexpected bundle ID; regenerate the project and rebuild.")
    if not has_device_family(info):
        raise RuntimeError("Unexpected app device family: " + DEVICE_FAMILY_ADVICE)
    # The family check above guarantees iPad is part of the build, so its orientations always matter.
    if not declares_ipad_orientations(info):
        raise RuntimeError("An iPad build must declare all four orientations in UISupportedInterfaceOrientations~ipad "
                           "or set UIRequiresFullScreen; Apple rejects the upload otherwise.")
    if info.get("AHPushEnvironment") != "prod" or info.get("ITSAppUsesNonExemptEncryption") is not False:
        raise RuntimeError("Review the production push environment and encryption declaration before distributing.")
    if not (app / "PrivacyInfo.xcprivacy").is_file():
        raise RuntimeError("The archive is missing its privacy manifest.")
    if not info.get("CFBundleShortVersionString") or not info.get("CFBundleVersion"):
        raise RuntimeError("The archive is missing its version or build number.")
    extension = plistlib.loads((app / "PlugIns/AskHumanNotification.appex/Info.plist").read_bytes())
    if extension.get("CFBundleIdentifier") != BUNDLE_ID + ".notification" or any(
        extension.get(key) != info[key] for key in ("CFBundleVersion", "CFBundleShortVersionString")
    ):
        raise RuntimeError("The notification extension must match the app's version and build.")
    if not has_device_family(extension):
        raise RuntimeError("Unexpected notification extension device family: " + DEVICE_FAMILY_ADVICE)
    return info["CFBundleShortVersionString"], info["CFBundleVersion"]


def prepare(archive, config_path, audience="external"):
    if audience not in ("internal", "external"):
        raise ValueError("Audience must be internal or external.")
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
    version, build = inspect_archive(archive)
    print(json.dumps({"archive": str(archive), "bundleId": BUNDLE_ID, "version": version, "build": build,
                      "credentialBackend": "keychain", "audience": audience,
                      "internalOnly": audience == "internal"}, indent=2), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--config", type=Path, default=Path.home() / ".config/askhuman-release/app-store-connect.json")
    parser.add_argument("--audience", choices=("internal", "external"), default="external",
                        help="Default: external, for both existing Internal and Friends groups. Internal is an explicit opt-out; upload alone does not submit or invite anyone.")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--upload", action="store_true", help="Upload for the configured TestFlight audience")
    mode.add_argument("--check-signing", action="store_true", help="Export locally without uploading to Apple")
    args = parser.parse_args()
    archive, config = args.archive.resolve(), args.config.expanduser().resolve()
    prepare(archive, config, args.audience)
    if args.upload or args.check_signing:
        return invoke("upload" if args.upload else "export", archive, config, audience=args.audience)
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

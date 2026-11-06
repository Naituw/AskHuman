#!/usr/bin/env python3
"""Build and notarize the universal Production helper without creating Apple account resources.

build uses an existing Developer ID identity and profile. notarize is a separate, explicit upload.
Credentials are passed by keychain profile or API-key file; they are never copied into the product.
"""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parent
BUNDLE = "com.naituw.humaninloop.todosync"
CONTAINER = "iCloud.com.naituw.humaninloop.todosync"
TEAM = "DMJXDB9H6Q"
APP = "AskHumanTodoSync.app"


def run(argv, *, capture=False):
    result = subprocess.run([str(v) for v in argv], check=True,
                            stdout=subprocess.PIPE if capture else None)
    return result.stdout if capture else None


def validate_entitlements(entitlements, *, profile=False):
    expected = {
        "com.apple.developer.icloud-container-environment": "Production",
        "com.apple.developer.aps-environment": "production",
    }
    for key, value in expected.items():
        if entitlements.get(key) != value:
            raise ValueError(f"helper requires {key}={value}")
    if CONTAINER not in entitlements.get("com.apple.developer.icloud-container-identifiers", []):
        raise ValueError("helper profile does not grant the todo sync container")
    services = entitlements.get("com.apple.developer.icloud-services", [])
    if "CloudKit" not in services and not (profile and "*" in services):
        raise ValueError("helper profile does not grant CloudKit")


def validate_profile(profile, now=None):
    if profile.get("TeamIdentifier") != [TEAM]:
        raise ValueError("helper profile has an unexpected team")
    entitlements = profile.get("Entitlements", {})
    if entitlements.get("com.apple.application-identifier") != f"{TEAM}.{BUNDLE}":
        raise ValueError("helper profile has an unexpected application identifier")
    validate_entitlements(entitlements, profile=True)
    now = now or datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)
    expires = profile.get("ExpirationDate")
    if not isinstance(expires, datetime.datetime) or expires.replace(tzinfo=None) <= now:
        raise ValueError("helper profile is expired or has no expiration date")
    for key in ("UUID", "Name"):
        if not isinstance(profile.get(key), str) or not profile[key].strip():
            raise ValueError(f"helper profile is missing {key}")


def verify(app, version, notarized=False):
    requirement = (f'=anchor apple generic and identifier "{BUNDLE}" and '
                   'certificate 1[field.1.2.840.113635.100.6.2.6] exists and '
                   'certificate leaf[field.1.2.840.113635.100.6.1.13] exists and '
                   f'certificate leaf[subject.OU] = "{TEAM}"')
    if notarized:
        requirement += " and notarized"
    run(["/usr/bin/codesign", "--verify", "--deep", "--strict", "-R", requirement, app])
    entitlements = plistlib.loads(run(["/usr/bin/codesign", "-d", "--entitlements", ":-", app], capture=True))
    validate_entitlements(entitlements)
    info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    if (info.get("CFBundleIdentifier") != BUNDLE
            or info.get("CFBundleShortVersionString") != version
            or str(info.get("AskHumanTodoSyncProtocol")) != "1"
            or info.get("LSMinimumSystemVersion") != "14.0"
            or info.get("HelperContainerID") != CONTAINER):
        raise ValueError("helper bundle identity, version, protocol or system requirement is incorrect")
    architectures = set(run(["/usr/bin/lipo", "-archs", app / "Contents/MacOS/AskHumanTodoSync"], capture=True).decode().split())
    if architectures != {"arm64", "x86_64"}:
        raise ValueError("helper must contain both arm64 and x86_64")
    if notarized:
        run(["/usr/bin/xcrun", "stapler", "validate", app])
        run(["/usr/sbin/spctl", "--assess", "--type", "execute", app])


def build(args):
    profile_path = args.profile.resolve()
    profile = plistlib.loads(run(["/usr/bin/security", "cms", "-D", "-i", profile_path], capture=True))
    validate_profile(profile)
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="askhuman-helper-build-") as temporary:
        temporary = Path(temporary)
        entitlements = {key: profile["Entitlements"][key] for key in (
            "com.apple.developer.icloud-container-identifiers",
            "com.apple.developer.icloud-services",
            "com.apple.developer.icloud-container-environment",
            "com.apple.developer.aps-environment")}
        # Profiles may grant wildcard services. Request only the service the helper actually uses.
        entitlements["com.apple.developer.icloud-services"] = ["CloudKit"]
        entitlements["com.apple.application-identifier"] = f"{TEAM}.{BUNDLE}"
        entitlements["com.apple.developer.team-identifier"] = TEAM
        entitlement_path = temporary / "release.entitlements"
        entitlement_path.write_bytes(plistlib.dumps(entitlements))
        run(["xcodegen", "generate", "--quiet", "--spec", ROOT / "project.yml"])
        archive = temporary / "helper.xcarchive"
        # Compile without account-dependent Xcode signing. In particular an existing Xcode-managed
        # Developer ID profile is valid for distribution but Xcode refuses it in manual-signing mode.
        # Embed that existing profile and sign explicitly after the build; no profiles are installed
        # in the user's Xcode directories and no automatic provisioning is requested.
        run(["xcodebuild", "-project", ROOT / "AskHumanTodoSync.xcodeproj", "-scheme", "AskHumanTodoSync",
             "-configuration", "Release", "-destination", "generic/platform=macOS",
             "-archivePath", archive, "-derivedDataPath", temporary / "DerivedData",
             "CODE_SIGNING_ALLOWED=NO", "CODE_SIGNING_REQUIRED=NO",
             "ARCHS=arm64 x86_64", "ONLY_ACTIVE_ARCH=NO",
             f"MARKETING_VERSION={args.version}", f"CURRENT_PROJECT_VERSION={args.version}", "archive"])
        app = args.output / APP
        if app.exists():
            raise ValueError("output already contains a helper app; select an empty output directory")
        run(["/usr/bin/ditto", archive / "Products/Applications" / APP, app])
        shutil.copyfile(profile_path, app / "Contents/embedded.provisionprofile")
        run(["/usr/bin/codesign", "--force", "--options", "runtime", "--timestamp", "--generate-entitlement-der",
             "--entitlements", entitlement_path, "--sign", args.identity, app])
        verify(app, args.version)


def notarize(args):
    if not args.allow_apple_upload:
        raise ValueError("notarization uploads to Apple; pass --allow-apple-upload after approval")
    app = args.app.resolve()
    verify(app, args.version)
    code_hash = hashlib.sha256((app / "Contents/MacOS/AskHumanTodoSync").read_bytes()).hexdigest()
    receipt_path = app.parent / f"notary-{code_hash}.json"
    receipt = json.loads(receipt_path.read_text()) if receipt_path.exists() else None
    with tempfile.TemporaryDirectory(prefix="askhuman-helper-notary-") as temporary:
        upload = Path(temporary) / "helper.zip"
        run(["/usr/bin/ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, upload])
        if args.keychain_profile:
            auth = ["--keychain-profile", args.keychain_profile]
            if getattr(args, "keychain", None):
                auth.extend(["--keychain", args.keychain])
        elif args.key and args.key_id and args.issuer:
            auth = ["--key", args.key, "--key-id", args.key_id, "--issuer", args.issuer]
        else:
            raise ValueError("provide a notary keychain profile, or API-key file, key id and issuer")
        if receipt is None:
            # Persist the id before waiting. A timeout or interrupted process must not cause a
            # duplicate upload of the same signed code on the next invocation.
            shutil.copyfile(upload, app.parent / "notary-input.zip")
            receipt = json.loads(run(["/usr/bin/xcrun", "notarytool", "submit", upload, *auth,
                                      "--output-format", "json"], capture=True))
            if not isinstance(receipt.get("id"), str) or not receipt["id"]:
                raise ValueError("notary service did not return a submission id")
            receipt = {"id": receipt["id"], "codeSha256": code_hash, "version": args.version}
            pending = receipt_path.with_suffix(".tmp")
            pending.write_text(json.dumps(receipt))
            pending.replace(receipt_path)
        if receipt.get("codeSha256") != code_hash or receipt.get("version") != args.version:
            raise ValueError("saved notarization submission does not match the signed candidate")
        print(json.dumps({"submission": receipt["id"], "receipt": str(receipt_path)}), flush=True)
        response = json.loads(run(["/usr/bin/xcrun", "notarytool", "wait", receipt["id"], *auth,
                                   "--timeout", "10m", "--output-format", "json"], capture=True))
        if response.get("status") != "Accepted":
            raise ValueError(f"notarization was not accepted (submission {response.get('id', 'unknown')})")
        receipt["status"] = "Accepted"
        receipt_path.write_text(json.dumps(receipt))
        run(["/usr/bin/xcrun", "stapler", "staple", app])
        verify(app, args.version, notarized=True)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        packaged = Path(temporary) / "AskHumanTodoSync.zip"
        run(["/usr/bin/ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, packaged])
        shutil.copyfile(packaged, args.output)
        print(json.dumps({"status": "Accepted", "submission": receipt["id"], "archive": str(args.output)}))


def main():
    version = json.loads((REPO / "package.json").read_text())["version"]
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    make = sub.add_parser("build")
    make.add_argument("--profile", type=Path, required=True)
    make.add_argument("--identity", required=True)
    make.add_argument("--output", type=Path, required=True)
    make.add_argument("--version", default=version)
    upload = sub.add_parser("notarize")
    upload.add_argument("--app", type=Path, required=True)
    upload.add_argument("--output", type=Path, required=True)
    upload.add_argument("--version", default=version)
    upload.add_argument("--allow-apple-upload", action="store_true")
    upload.add_argument("--keychain-profile")
    upload.add_argument("--keychain", type=Path)
    upload.add_argument("--key", type=Path)
    upload.add_argument("--key-id")
    upload.add_argument("--issuer")
    check = sub.add_parser("verify")
    check.add_argument("--app", type=Path, required=True)
    check.add_argument("--version", default=version)
    args = parser.parse_args()
    try:
        if args.command == "build":
            build(args)
        elif args.command == "notarize":
            notarize(args)
        else:
            verify(args.app, args.version, notarized=True)
    except (ValueError, subprocess.CalledProcessError, OSError) as error:
        parser.exit(1, f"todo sync release: {error}\n")


if __name__ == "__main__":
    main()

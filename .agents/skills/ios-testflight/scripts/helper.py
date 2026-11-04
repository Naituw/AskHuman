#!/usr/bin/env python3
"""Install or invoke the signed TestFlight operation helper in the user's GUI session."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid

IDENTIFIER = "com.naituw.askhuman.testflight-helper"
IDENTITY = "Developer ID Application: Wu Tian (DMJXDB9H6Q)"
HELPER_SOURCES = ("TestFlightHelper.swift", "NotesRules.swift")

# Rules for the `notes` operation. NotesRules.swift enforces the same rules inside the helper;
# notes-rules-cases.json is the contract both implementations must reproduce.
NOTES_LOCALES = ("en-US", "zh-Hans")
NOTES_MAX_LENGTH = 4000  # UTF-16 code units
NOTES_MODES = ("read", "write", "probe")


class NotesRejected(ValueError):
    """A notes request the helper would refuse. `reason` is the code the shared cases assert."""

    def __init__(self, reason, message):
        super().__init__(message)
        self.reason = reason


def notes_canonicalize(text):
    """Normalize line endings, reject control characters, strip the trailing run of spaces and newlines."""
    value = text.replace("\r\n", "\n").replace("\r", "\n")
    if any((ord(char) < 0x20 and char != "\n") or ord(char) == 0x7F for char in value):
        raise NotesRejected("control", "Notes may not contain control characters other than newlines.")
    value = value.rstrip(" \n")
    if not value:
        raise NotesRejected("empty", "The notes text is empty.")
    try:
        units = len(value.encode("utf-16-le")) // 2
    except UnicodeEncodeError:
        raise NotesRejected("badRequest", "The notes text is not valid Unicode.") from None
    if units > NOTES_MAX_LENGTH:
        raise NotesRejected("tooLong", f"The notes text is {units} UTF-16 units; the limit is {NOTES_MAX_LENGTH}.")
    return value


def notes_validate_build(build):
    if not isinstance(build, str) or re.fullmatch(r"[1-9][0-9]{0,8}", build) is None:
        raise NotesRejected("badBuild", "The build number must be 1-9 ASCII digits without a leading zero.")


def notes_request(request):
    """Validate a decoded request object and return it with canonical texts."""
    if not isinstance(request, dict) or not set(request) <= {"mode", "build", "texts"}:
        raise NotesRejected("badRequest", "The notes request must be an object with only mode, build and texts.")
    mode = request.get("mode")
    if not isinstance(mode, str) or mode not in NOTES_MODES:
        raise NotesRejected("badMode", "The mode must be read, write or probe.")
    build = request.get("build")
    notes_validate_build(build)
    if mode != "write":
        if "texts" in request:
            raise NotesRejected("badRequest", "Only write requests carry texts.")
        return {"mode": mode, "build": build, "texts": {}}
    raw = request.get("texts")
    if not isinstance(raw, dict) or not raw:
        raise NotesRejected("badRequest", "A write request needs a non-empty texts object.")
    for locale in sorted(raw):
        if locale not in NOTES_LOCALES:
            raise NotesRejected("badLocale", f"Only {', '.join(NOTES_LOCALES)} are supported.")
    texts = {}
    for locale in sorted(raw):
        if not isinstance(raw[locale], str):
            raise NotesRejected("badRequest", "Each text must be a string.")
        texts[locale] = notes_canonicalize(raw[locale])
    return {"mode": mode, "build": build, "texts": texts}


def notes_wire(request):
    """The request as the helper reads it: only a write carries texts, and the helper refuses an empty texts key."""
    wire = {"mode": request["mode"], "build": request["build"]}
    if request["mode"] == "write":
        wire["texts"] = request["texts"]
    return wire


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
        sources = [str(Path(__file__).with_name(name)) for name in HELPER_SOURCES]
        subprocess.run(["xcrun", "swiftc", "-parse-as-library", "-O", *sources, "-o", str(binary)], check=True)
        subprocess.run(["/usr/bin/codesign", "--force", "--timestamp=none", "-i", IDENTIFIER, "--sign", IDENTITY, str(binary)], check=True)
        verify(binary)
        binary.chmod(0o700)
        target = root / "testflight-helper"
        kept = False
        if target.exists():
            # Keep the helper being replaced so a bad update can be undone by renaming this file back.
            backup = Path(folder) / "testflight-helper.previous"
            shutil.copy2(target, backup)
            verify(backup)
            backup.chmod(0o700)
            os.replace(backup, root / "testflight-helper.previous")
            kept = True
        os.replace(binary, target)
    print("Installed the stable signed TestFlight helper; no credentials were displayed."
          + (" The previous helper is kept as testflight-helper.previous." if kept else ""))


def invoke(action, archive, config, request=None):
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
        if request is not None:
            # The request travels in an owner-only file inside this throwaway directory, never in argv or the environment.
            request_file = folder / "request.json"
            descriptor = os.open(request_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
                json.dump(request, handle, ensure_ascii=False)
            args.append(str(request_file))
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


def read_note_file(path):
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise NotesRejected("badRequest", f"Cannot read {path} as UTF-8 text: {error}") from None


def dry_run_report(request):
    """What a real run would send. Nothing here starts the helper, reads credentials, or touches the network."""
    mode = request["mode"]
    calls = ["GET /v1/builds (this app, this build number; must be VALID and not expired)",
             "GET /v1/betaBuildLocalizations (this build)"]
    if mode == "write":
        calls += ["POST /v1/betaBuildLocalizations (for each locale without an entry)",
                  "PATCH /v1/betaBuildLocalizations/<id> (for each locale whose text differs)",
                  "GET /v1/betaBuildLocalizations (read-back; must match exactly)"]
    elif mode == "probe":
        calls += ["POST /v1/betaBuildLocalizations (a marker text, for each locale without an entry)",
                  "GET /v1/betaBuildLocalizations (read-back)",
                  "DELETE /v1/betaBuildLocalizations/<id created by this run>",
                  "GET /v1/betaBuildLocalizations (must no longer list it)"]
    return {"dryRun": True, "mode": mode, "build": request["build"],
            "locales": {locale: {"length": len(text.encode("utf-16-le")) // 2} for locale, text in request["texts"].items()},
            "wouldCall": calls}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["install", "migrate", "status", "export", "upload", "notes"])
    parser.add_argument("archive", nargs="?", type=Path)
    parser.add_argument("--config", type=Path, default=Path.home() / ".config/askhuman-release/app-store-connect.json")
    notes = parser.add_argument_group("notes", "read, write or probe a build's TestFlight test information")
    notes.add_argument("--build", help="build number (CFBundleVersion)")
    notes.add_argument("--mode", choices=NOTES_MODES, help="read: show it; write: set it; probe: write a marker to an empty locale, read it back, delete it")
    notes.add_argument("--zh", type=Path, help="zh-Hans text file (write)")
    notes.add_argument("--en", type=Path, help="en-US text file (write)")
    notes.add_argument("--dry-run", action="store_true", help="validate and describe the request without contacting anything")
    args = parser.parse_args()
    config = args.config.expanduser().resolve()
    if args.action != "notes" and any([args.build, args.mode, args.zh, args.en, args.dry_run]):
        parser.error("--build, --mode, --zh, --en and --dry-run only apply to the notes action.")
    if args.action == "install":
        install(config.parent)
        return 0
    if args.action == "notes":
        if args.archive is not None:
            parser.error("notes takes no archive path.")
        if args.build is None or args.mode is None:
            parser.error("notes requires --build and --mode.")
        raw = {"mode": args.mode, "build": args.build}
        if args.mode == "write":
            texts = {locale: read_note_file(path) for locale, path in (("zh-Hans", args.zh), ("en-US", args.en)) if path is not None}
            if not texts:
                parser.error("A write needs --zh and/or --en.")
            raw["texts"] = texts
        elif args.zh or args.en:
            parser.error("--zh and --en only apply to --mode write.")
        try:
            request = notes_request(raw)
        except NotesRejected as error:
            print(f"TestFlight notes: {error.reason}: {error}", file=sys.stderr)
            return 1
        if args.dry_run:
            print(json.dumps(dry_run_report(request), ensure_ascii=False, indent=2))
            return 0
        return invoke("notes", None, config, request=notes_wire(request))
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

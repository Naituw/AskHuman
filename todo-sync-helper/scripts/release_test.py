import datetime
import importlib.util
from pathlib import Path
import unittest
from unittest import mock
import json
import subprocess
import tempfile
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location("helper_release", Path(__file__).with_name("release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def profile(self):
        return {
            "Name": "helper fixture", "UUID": "fixture-id", "TeamIdentifier": [release.TEAM],
            "ExpirationDate": datetime.datetime(2099, 1, 1),
            "Entitlements": {
                "com.apple.application-identifier": f"{release.TEAM}.{release.BUNDLE}",
                "com.apple.developer.icloud-container-environment": "Production",
                "com.apple.developer.aps-environment": "production",
                "com.apple.developer.icloud-container-identifiers": [release.CONTAINER],
                "com.apple.developer.icloud-services": ["CloudKit"],
            },
        }

    def test_production_profile_is_accepted_without_contacting_apple(self):
        release.validate_profile(self.profile())

    def test_profile_wildcard_does_not_allow_a_wildcard_signed_binary(self):
        profile = self.profile()
        profile["Entitlements"]["com.apple.developer.icloud-services"] = ["*"]
        release.validate_profile(profile)
        with self.assertRaises(ValueError):
            release.validate_entitlements(profile["Entitlements"])

    def test_development_environment_and_sandbox_push_are_rejected(self):
        for key, value in [
            ("com.apple.developer.icloud-container-environment", "Development"),
            ("com.apple.developer.aps-environment", "development"),
            ("com.apple.developer.icloud-container-identifiers", ["iCloud.wrong"]),
            ("com.apple.developer.icloud-services", []),
        ]:
            with self.subTest(key=key):
                profile = self.profile()
                profile["Entitlements"][key] = value
                with self.assertRaises(ValueError):
                    release.validate_profile(profile)

    def test_wrong_publisher_and_application_are_rejected(self):
        profile = self.profile()
        profile["TeamIdentifier"] = ["OTHER"]
        with self.assertRaises(ValueError):
            release.validate_profile(profile)
        profile = self.profile()
        profile["Entitlements"]["com.apple.application-identifier"] = "OTHER.app"
        with self.assertRaises(ValueError):
            release.validate_profile(profile)

    def test_expired_or_incomplete_profile_is_rejected(self):
        for key in ("Name", "UUID", "ExpirationDate"):
            profile = self.profile()
            del profile[key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                release.validate_profile(profile)
        profile = self.profile()
        profile["ExpirationDate"] = datetime.datetime(2000, 1, 1)
        with self.assertRaises(ValueError):
            release.validate_profile(profile)

    def test_notarization_requires_explicit_upload_intent(self):
        class Args:
            allow_apple_upload = False
        with self.assertRaisesRegex(ValueError, "uploads to Apple"):
            release.notarize(Args())

    def test_interrupted_notarization_resumes_the_same_submission(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            app = root / release.APP
            executable = app / "Contents/MacOS/AskHumanTodoSync"
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"signed code fixture")
            args = SimpleNamespace(allow_apple_upload=True, app=app, version="0.16.0",
                                   keychain_profile="fixture", key=None, key_id=None, issuer=None,
                                   output=root / "output.zip")
            submits = []
            waits = []

            def runner(argv, *, capture=False):
                if argv[0] == "/usr/bin/ditto":
                    Path(argv[-1]).write_bytes(b"archive fixture")
                elif argv[1:3] == ["notarytool", "submit"]:
                    submits.append(argv)
                    return json.dumps({"id": "submission-fixture"}).encode()
                elif argv[1:3] == ["notarytool", "wait"]:
                    waits.append(argv)
                    if len(waits) == 1:
                        raise subprocess.CalledProcessError(75, argv)
                    return json.dumps({"id": "submission-fixture", "status": "Accepted"}).encode()
                return None

            with mock.patch.object(release, "run", side_effect=runner), mock.patch.object(release, "verify"):
                with self.assertRaises(subprocess.CalledProcessError):
                    release.notarize(args)
                receipts = list(root.glob("notary-*.json"))
                self.assertEqual(len(receipts), 1)
                self.assertEqual(json.loads(receipts[0].read_text())["id"], "submission-fixture")
                release.notarize(args)
            self.assertEqual(len(submits), 1)
            self.assertEqual(len(waits), 2)
            self.assertEqual(waits[0][3], waits[1][3])
            self.assertTrue(args.output.exists())


if __name__ == "__main__":
    unittest.main()

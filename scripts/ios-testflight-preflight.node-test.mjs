// Offline tests for the ios-testflight archive preflight (`inspect_archive` in `upload.py`): the shipped device
// family (iPhone and iPad), the iPad orientation declaration, and the rules that were already there.
// Each case builds a synthetic archive in a temporary directory; nothing reads a credential, starts the helper,
// contacts Apple, or needs a real archive. The Swift side of the device-family rule lives inline in
// `TestFlightHelper.swift` and is verified against real archives; only its agreement with Python is guarded here.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const scriptsDir = fileURLToPath(new URL("../.agents/skills/ios-testflight/scripts/", import.meta.url));

const FOUR = [
  "UIInterfaceOrientationPortrait",
  "UIInterfaceOrientationPortraitUpsideDown",
  "UIInterfaceOrientationLandscapeLeft",
  "UIInterfaceOrientationLandscapeRight",
];
const PORTRAIT = ["UIInterfaceOrientationPortrait"];

// What a real archive of the current project declares (checked against the build-7 candidate on 2026-11-04).
const APP = {
  CFBundleIdentifier: "com.naituw.humaninloop.ios",
  CFBundleShortVersionString: "0.1.0",
  CFBundleVersion: "7",
  UIDeviceFamily: [1, 2],
  UISupportedInterfaceOrientations: PORTRAIT,
  "UISupportedInterfaceOrientations~ipad": FOUR,
  AHPushEnvironment: "prod",
  ITSAppUsesNonExemptEncryption: false,
};
const EXTENSION = {
  CFBundleIdentifier: "com.naituw.humaninloop.ios.notification",
  CFBundleShortVersionString: "0.1.0",
  CFBundleVersion: "7",
  UIDeviceFamily: [1, 2],
};

// `null` deletes a key; any other value replaces it.
function derive(base, changes) {
  const result = { ...base, ...changes };
  for (const [key, value] of Object.entries(result)) if (value === null) delete result[key];
  return result;
}

// JSON cannot tell 1.0 from 1, so a real number travels as { $real: n } and becomes a plist <real> in Python.
const real = (value) => ({ $real: value });

const PASS = "pass";

// [name, app changes, extension changes, expectation: PASS or a pattern the error must match, options]
const CASES = [
  // ── accepted ──
  ["the current project: iPhone and iPad, four iPad orientations", {}, {}, PASS],
  ["the order of the pair does not matter", { UIDeviceFamily: [2, 1] }, { UIDeviceFamily: [2, 1] }, PASS],
  ["the generic list is the fallback when there is no iPad-specific one",
    { UISupportedInterfaceOrientations: FOUR, "UISupportedInterfaceOrientations~ipad": null }, {}, PASS],
  ["opting out of multitasking replaces the four orientations",
    { "UISupportedInterfaceOrientations~ipad": PORTRAIT, UIRequiresFullScreen: true }, {}, PASS],
  ["opting out of multitasking also works with no orientation keys at all",
    { UISupportedInterfaceOrientations: null, "UISupportedInterfaceOrientations~ipad": null, UIRequiresFullScreen: true }, {}, PASS],

  // ── device family of the app ──
  ["an iPhone-only app is refused", { UIDeviceFamily: [1] }, { UIDeviceFamily: [1] }, /app device family/],
  ["an iPad-only app is refused", { UIDeviceFamily: [2] }, {}, /app device family/],
  ["a repeated family is refused", { UIDeviceFamily: [1, 2, 2] }, {}, /app device family/],
  ["an extra family is refused", { UIDeviceFamily: [1, 2, 3] }, {}, /app device family/],
  ["an empty family list is refused", { UIDeviceFamily: [] }, {}, /app device family/],
  ["a missing family is refused", { UIDeviceFamily: null }, {}, /app device family/],
  ["a family that is not a list is refused", { UIDeviceFamily: "1,2" }, {}, /app device family/],
  ["strings are not families", { UIDeviceFamily: ["1", "2"] }, {}, /app device family/],
  ["booleans are not families", { UIDeviceFamily: [true, 2] }, {}, /app device family/],
  ["reals are not families", { UIDeviceFamily: [real(1), 2] }, {}, /app device family/],

  // ── device family of the extension ──
  ["an iPhone-only extension is refused", {}, { UIDeviceFamily: [1] }, /extension device family/],
  ["an iPad-only extension is refused", {}, { UIDeviceFamily: [2] }, /extension device family/],
  ["a missing extension family is refused", {}, { UIDeviceFamily: null }, /extension device family/],
  ["a repeated extension family is refused", {}, { UIDeviceFamily: [1, 1, 2] }, /extension device family/],
  ["the advice names what to change and where",
    { UIDeviceFamily: [1] }, {}, /1,2.*upload\.py.*TestFlightHelper\.swift.*reinstall the helper.*TARGETED_DEVICE_FAMILY/s],

  // ── iPad orientations ──
  ["three iPad orientations are refused", { "UISupportedInterfaceOrientations~ipad": FOUR.slice(0, 3) }, {}, /four orientations/],
  ["the iPad-specific list wins over a complete generic one",
    { UISupportedInterfaceOrientations: FOUR, "UISupportedInterfaceOrientations~ipad": PORTRAIT }, {}, /four orientations/],
  ["no orientation keys at all are refused",
    { UISupportedInterfaceOrientations: null, "UISupportedInterfaceOrientations~ipad": null }, {}, /four orientations/],
  ["an iPhone-only portrait list is not enough for iPad",
    { "UISupportedInterfaceOrientations~ipad": null }, {}, /four orientations/],
  ["UIRequiresFullScreen set to false does not opt out",
    { "UISupportedInterfaceOrientations~ipad": FOUR.slice(0, 3), UIRequiresFullScreen: false }, {}, /four orientations/],
  ["UIRequiresFullScreen must be a real boolean",
    { "UISupportedInterfaceOrientations~ipad": PORTRAIT, UIRequiresFullScreen: "YES" }, {}, /four orientations/],
  ["an orientation list that is not a list is refused",
    { "UISupportedInterfaceOrientations~ipad": "all" }, {}, /four orientations/],
  ["malformed orientation entries are refused rather than crashing the check",
    { "UISupportedInterfaceOrientations~ipad": [1, { a: 1 }, [FOUR[0]]] }, {}, /four orientations/],

  // ── rules that were already there ──
  ["the app bundle ID must match", { CFBundleIdentifier: "com.example.other" }, {}, /bundle ID/],
  ["the push environment must be production", { AHPushEnvironment: "dev" }, {}, /push environment/],
  ["the encryption declaration is required", { ITSAppUsesNonExemptEncryption: null }, {}, /encryption/],
  ["the privacy manifest is required", {}, {}, /privacy manifest/, { privacyManifest: false }],
  ["a version is required", { CFBundleShortVersionString: "" }, {}, /version or build number/],
  ["the extension must carry the app's build", {}, { CFBundleVersion: "6" }, /extension must match the app's version and build/],
  ["the extension must carry the app's version", {}, { CFBundleShortVersionString: "0.2.0" }, /extension must match the app's version and build/],
  ["the extension bundle ID must match", {}, { CFBundleIdentifier: "com.naituw.humaninloop.ios.other" }, /extension must match the app's version and build/],
];

const PYTHON_RUN = String.raw`
import json, pathlib, plistlib, sys, tempfile
sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
import upload

def expand(value):
    if isinstance(value, dict):
        if set(value) == {"$real"}:
            return float(value["$real"])
        return {key: expand(item) for key, item in value.items()}
    if isinstance(value, list):
        return [expand(item) for item in value]
    return value

results = []
for case in json.load(sys.stdin):
    with tempfile.TemporaryDirectory() as tmp:
        app = pathlib.Path(tmp) / "Products/Applications/AskHuman.app"
        extension = app / "PlugIns/AskHumanNotification.appex"
        extension.mkdir(parents=True)
        (app / "Info.plist").write_bytes(plistlib.dumps(expand(case["app"])))
        (extension / "Info.plist").write_bytes(plistlib.dumps(expand(case["extension"])))
        if case["privacyManifest"]:
            (app / "PrivacyInfo.xcprivacy").write_bytes(plistlib.dumps({}))
        try:
            results.append({"name": case["name"], "ok": True, "result": list(upload.inspect_archive(pathlib.Path(tmp)))})
        except RuntimeError as error:
            results.append({"name": case["name"], "ok": False, "error": str(error)})
print(json.dumps(results))
`;

function runCases() {
  const input = CASES.map(([name, appChanges, extensionChanges, , options = {}]) => ({
    name,
    app: derive(APP, appChanges),
    extension: derive(EXTENSION, extensionChanges),
    privacyManifest: options.privacyManifest ?? true,
  }));
  const run = spawnSync("python3", ["-B", "-c", PYTHON_RUN, scriptsDir], { encoding: "utf8", input: JSON.stringify(input) });
  assert.equal(run.status, 0, run.stderr);
  return JSON.parse(run.stdout);
}

test("inspect_archive accepts and refuses exactly what the preflight rules say", () => {
  const results = runCases();
  assert.equal(results.length, CASES.length);
  results.forEach((result, index) => {
    const [name, , , expectation] = CASES[index];
    assert.equal(result.name, name);
    if (expectation === PASS) {
      assert.equal(result.ok, true, `${name}: refused with "${result.error}"`);
      assert.deepEqual(result.result, ["0.1.0", "7"], name);
    } else {
      assert.equal(result.ok, false, `${name}: should have been refused`);
      assert.match(result.error, expectation, name);
    }
  });
});

test("the cases cover both outcomes and every kind of rule", () => {
  const accepted = CASES.filter(([, , , expectation]) => expectation === PASS).length;
  assert.ok(accepted >= 4 && CASES.length - accepted >= 25, "keep both sides well covered");
  for (const kind of [/app device family/, /extension device family/, /four orientations/, /bundle ID/, /push environment/, /privacy manifest/]) {
    assert.ok(CASES.some(([, , , expectation]) => String(expectation) === String(kind)), `no case for ${kind}`);
  }
});

test("upload.py and the signed helper ship to the same pair of device families", () => {
  const python = spawnSync("python3", ["-B", "-c", "import sys, json; sys.dont_write_bytecode = True; sys.path.insert(0, sys.argv[1]); import upload; print(json.dumps(upload.DEVICE_FAMILY))", scriptsDir], { encoding: "utf8" });
  assert.equal(python.status, 0, python.stderr);
  assert.deepEqual(JSON.parse(python.stdout), [1, 2]);

  const swift = readFileSync(join(scriptsDir, "TestFlightHelper.swift"), "utf8");
  assert.match(swift, /numbers\.map\(\\\.intValue\)\.sorted\(\) == \[1, 2\]/, "the Swift rule must be the same strict pair");
  assert.doesNotMatch(swift, /UIDeviceFamily"\] as\? \[Int\]/, "the old exact-list comparison must not come back");
  assert.equal(swift.match(/isShippedDeviceFamily\(/g).length, 3, "one definition plus the app and extension checks");
  assert.match(swift, /isShippedDeviceFamily\(info\["UIDeviceFamily"\]\)/);
  assert.match(swift, /isShippedDeviceFamily\(notification\["UIDeviceFamily"\]\)/);
});

# Attachment drag receiver

This simulator-only app consumes a real cross-process `NSItemProvider` file promise, copies its bytes to Documents and records the filename, type, size and SHA-256 in `receipt.txt`. It needs no account and sends nothing externally. It is not included in AskHuman's app scheme.

Generate and build this directory's XcodeGen project, install its Debug simulator app, and build AskHuman for testing. Follow the repository machine-health check before each build or test.

Run the host launcher in parallel with the UI tests, using the same simulator UDID:

```sh
python3 ios/Tools/drag-receiver/launch-during-drag.py --device SIMULATOR_UDID
```

```sh
TEST_RUNNER_AH_DRAG_RECEIVER=1 xcodebuild test-without-building \
  -project ios/AskHuman.xcodeproj -scheme AskHuman \
  -destination 'platform=iOS Simulator,id=SIMULATOR_UDID' \
  -derivedDataPath ios/build/DerivedDataEnded \
  -only-testing:AskHumanUITests/AskHumanAttachmentDragUITests \
  -parallel-testing-enabled NO
```

The Debug mock-only `-AHDragReceiverTest YES` signal is written when the real drag provider is created. The launcher then switches apps while XCTest keeps its finger down, independently of accessibility-query scheduling delays. A test validates delivery, rather than just finding a drag action. For one case use `--count 1`; `--delay` is the pause after lift. Compare receiver Documents with AskHuman's `tmp/ah-mock-assets` to verify bytes. Terminate and uninstall the receiver when finished. Files, Notes and real chat apps still need their own compatibility checks where available.

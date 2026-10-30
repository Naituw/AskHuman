# AskHuman Tao patch

This is crates.io `tao` **0.35.3**, with its original Apache-2.0 license. Only
`src/platform_impl/macos/window.rs` and `window_delegate.rs` differ from that release.

The macOS `drag_window()` implementation used `NSApp.currentEvent` after asynchronous WebView
IPC. Its `0x15` fallback built a window-associated event with `NSEvent.mouseLocation`, a screen
coordinate, as the window-local location. AppKit then received the wrong drag offset; subtracting
it from the pointer position produces the global origin `(0, 0)` instead of the window origin.

The patch retains the original left-button-down event before the native window dispatches it to
WebKit. The latest primary gesture is bound to its window, consumed once by `drag_window()`, and
cleared on mouse-up or window close. A released button or a missing event cannot start a drag.
Native background dragging consumes the gesture too. All event ownership stays on the UI thread,
and no cache borrow is held while AppKit runs. Other platforms are unchanged.

Regression: `cargo run --manifest-path src-tauri/Cargo.toml --example popup_window_drag_regression --features custom-protocol`.

Keep the version pinned to the vendored release. Remove this patch once the upstream version used
by Tauri preserves a valid original mouse-down event across IPC and passes this regression.

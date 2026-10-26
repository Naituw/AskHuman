// Copyright 2023-2023 CrabNebula Ltd.
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use core_graphics::display::CGDisplay;
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{NSObject, NSObjectProtocol, ProtocolObject},
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_foundation::{NSArray, NSData, NSMutableArray, NSPoint, NSRect, NSSize, NSString, NSURL};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{CursorPosition, DragItem, DragMode, DragResult, Image, Options};
use objc2_app_kit::{
    NSApp, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSEvent,
    NSEventModifierFlags, NSEventType, NSImage, NSPasteboardItem, NSPasteboardItemDataProvider,
    NSView,
};

type OnDropCallback = Box<dyn Fn(DragResult, CursorPosition) + Send>;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DragRsDataProvider"]
    #[ivars = DragRsDataProviderIvars]
    struct DragRsDataProvider;

    unsafe impl NSObjectProtocol for DragRsDataProvider {}

    unsafe impl NSPasteboardItemDataProvider for DragRsDataProvider {
        #[unsafe(method(pasteboard:item:provideDataForType:))]
        unsafe fn provide_data(
            &self,
            _pasteboard: &objc2_app_kit::NSPasteboard,
            item: &NSPasteboardItem,
            data_type: &NSString,
        ) {
            let ivars = self.ivars();
            let provider = &ivars.provider;

            if let Some(data) = provider(&data_type.to_string()) {
                let ns_data = NSData::from_vec(data);
                let _ = item.setData_forType(&ns_data, data_type);
            }
        }
    }
);

struct DragRsDataProviderIvars {
    provider: crate::DataProvider,
}

impl DragRsDataProvider {
    pub fn new(provider: crate::DataProvider, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DragRsDataProviderIvars { provider });
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DragRsSource"]
    #[ivars = DragRsSourceIvars]
    struct DragRsSource;

    unsafe impl NSObjectProtocol for DragRsSource {}

    unsafe impl NSDraggingSource for DragRsSource {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        unsafe fn dragging_session(
            &self,
            session: &NSDraggingSession,
            _context: NSDraggingContext,
        ) -> objc2_app_kit::NSDragOperation {
            let ivars = self.ivars();
            session
                .setAnimatesToStartingPositionsOnCancelOrFail(ivars.animate_on_cancel_or_failure);

            ivars.drag_mode.into()
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        unsafe fn dragging_session_end(
            &self,
            _session: &NSDraggingSession,
            ended_at_point: NSPoint,
            operation: objc2_app_kit::NSDragOperation,
        ) {
            let callback = &self.ivars().on_drop_callback;

            let mouse_location = CursorPosition {
                x: ended_at_point.x as i32,
                y: CGDisplay::main().pixels_high() as i32 - ended_at_point.y as i32,
            };

            let callback_closure = callback.as_ref();

            if operation == objc2_app_kit::NSDragOperation::None {
                callback_closure(DragResult::Cancel, mouse_location);
            } else {
                callback_closure(DragResult::Dropped, mouse_location);
            }
        }
    }
);

struct DragRsSourceIvars {
    on_drop_callback: OnDropCallback,
    animate_on_cancel_or_failure: bool,
    drag_mode: DragMode,
}

impl DragRsSource {
    pub fn new<F: Fn(DragResult, CursorPosition) + Send + 'static>(
        on_drop_callback: F,
        options: &Options,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let on_drop_callback: OnDropCallback = Box::new(on_drop_callback);

        let this = Self::alloc(mtm).set_ivars(DragRsSourceIvars {
            on_drop_callback,
            animate_on_cancel_or_failure: !options.skip_animatation_on_cancel_or_failure,
            drag_mode: options.mode,
        });
        unsafe { msg_send![super(this), init] }
    }
}

pub fn start_drag<W: HasWindowHandle, F: Fn(DragResult, CursorPosition) + Send + 'static>(
    handle: &W,
    item: DragItem,
    image: Image,
    on_drop_callback: F,
    options: Options,
) -> crate::Result<()> {
    if let Ok(RawWindowHandle::AppKit(w)) = handle.window_handle().map(|h| h.as_raw()) {
        unsafe {
            let mtm = MainThreadMarker::new_unchecked();
            let ns_view = &*(w.ns_view.as_ptr() as *const NSView);
            let window = ns_view.window().expect("Failed to get window");
            let content_view = window.contentView().expect("Failed to get contentView");

            let current_position: NSPoint = window.mouseLocationOutsideOfEventStream();

            let img = match image {
                Image::File(path) => {
                    if !path.exists() {
                        return Err(crate::Error::ImageNotFound);
                    }
                    NSImage::initByReferencingFile(
                        NSImage::alloc(),
                        &NSString::from_str(&path.to_string_lossy()),
                    )
                }
                Image::Raw(bytes) => {
                    let data = NSData::from_vec(bytes);
                    NSImage::initWithData(NSImage::alloc(), &data)
                }
            };
            let img = img.expect("Failed to create NSImage");
            let image_size: NSSize = img.size();
            let image_rect = NSRect::new(
                NSPoint::new(
                    current_position.x - image_size.width / 2.,
                    current_position.y - image_size.height / 2.,
                ),
                image_size,
            );

            let dragging_items = NSMutableArray::new();

            match item {
                DragItem::Files(files) => {
                    for path in files {
                        let nsurl = NSURL::fileURLWithPath_isDirectory(
                            &NSString::from_str(&path.display().to_string()),
                            false,
                        );
                        let item = NSDraggingItem::initWithPasteboardWriter(
                            NSDraggingItem::alloc(),
                            &ProtocolObject::from_retained(nsurl),
                        );
                        item.setDraggingFrame_contents(image_rect, Some(&*img));
                        dragging_items.addObject(&*item);
                    }
                }
                DragItem::Data { provider, types } => {
                    let data_provider = DragRsDataProvider::new(provider, mtm);

                    let item = NSPasteboardItem::new();
                    let types_array = types
                        .into_iter()
                        .map(|t| NSString::from_str(&t))
                        .collect::<Vec<_>>();
                    let types_array = NSArray::from_retained_slice(&types_array);

                    item.setDataProvider_forTypes(
                        &ProtocolObject::from_retained(data_provider),
                        &types_array,
                    );

                    let drag_item = NSDraggingItem::initWithPasteboardWriter(
                        NSDraggingItem::alloc(),
                        &ProtocolObject::from_retained(item),
                    );
                    drag_item.setDraggingFrame_contents(image_rect, Some(&*img));
                    dragging_items.addObject(&*drag_item);
                }
            }

            let current_event = NSApp(mtm).currentEvent();
            let timestamp = current_event.map(|e| e.timestamp()).unwrap_or(0.0);
            let window_number = window.windowNumber();

            let drag_event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                NSEventType::LeftMouseDragged,
                current_position,
                NSEventModifierFlags::empty(),
                timestamp,
                window_number,
                None,
                0,
                1,
                1.0
            ).expect("Failed to create NSEvent");

            let source = DragRsSource::new(on_drop_callback, &options, mtm);

            let _ = content_view.beginDraggingSessionWithItems_event_source(
                &dragging_items,
                &drag_event,
                &ProtocolObject::<dyn NSDraggingSource>::from_retained(source),
            );

            Ok(())
        }
    } else {
        Err(crate::Error::UnsupportedWindowHandle)
    }
}

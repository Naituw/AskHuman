// Verification only: compile with clang -fobjc-arc -framework AppKit. No real pointer input.
#import <AppKit/AppKit.h>

@interface OffsetProbeWindow : NSWindow
@property NSPoint capturedOffset;
@end
@implementation OffsetProbeWindow
// Stop before asking WindowServer to drag. AppKit still performs its real event-to-window conversion.
- (void)_dragWindowRelativeToMouseDown:(NSPoint)point options:(NSUInteger)options completionHandler:(id)handler {
    self.capturedOffset = point;
}
@end

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        OffsetProbeWindow *w = [[OffsetProbeWindow alloc] initWithContentRect:NSMakeRect(0,0,480,420)
            styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskFullSizeContentView
            backing:NSBackingStoreBuffered defer:NO];
        NSMutableArray *rows = [NSMutableArray array];
        NSArray *origins = @[@[@(-2000),@200], @[@400,@200], @[@2400,@200],
                             @[@200,@1400], @[@(-1500),@(-900)], @[@0,@0]];
        NSUInteger checks = 0;
        for (NSArray *originPair in origins) {
            NSRect frame = NSMakeRect([originPair[0] doubleValue],[originPair[1] doubleValue],480,420);
            [w setFrame:frame display:NO];
          for (NSValue *click in @[[NSValue valueWithPoint:NSMakePoint(120,410)],
                                   [NSValue valueWithPoint:NSMakePoint(12,400)],
                                   [NSValue valueWithPoint:NSMakePoint(400,390)]]) {
            NSPoint local = click.pointValue;
            NSPoint screen = [w convertPointToScreen:local];
            for (NSString *mode in @[@"original",@"tao-0x15",@"screen-window-zero",@"corrected-fallback"]) {
                BOOL original = [mode isEqualToString:@"original"];
                NSInteger number = [mode isEqualToString:@"screen-window-zero"] ? 0 : w.windowNumber;
                NSPoint location = original ? local : screen;
                if ([mode isEqualToString:@"corrected-fallback"]) location = [w convertPointFromScreen:screen];
                NSEvent *e = [NSEvent mouseEventWithType:NSEventTypeLeftMouseDown
                    location:location modifierFlags:0 timestamp:NSProcessInfo.processInfo.systemUptime
                    windowNumber:number context:nil eventNumber:1 clickCount:1 pressure:1];
                w.capturedOffset = NSMakePoint(NAN,NAN);
                [w performWindowDragWithEvent:e];
                NSPoint offset = w.capturedOffset;
                // Captured at AppKit's entry to drag tracking, in the window's base coordinates.
                NSPoint origin = NSMakePoint(screen.x-offset.x, screen.y-offset.y);
                NSPoint expected = [mode isEqualToString:@"tao-0x15"] ? NSZeroPoint : w.frame.origin;
                if (!isfinite(offset.x) || !isfinite(offset.y) || fabs(origin.x-expected.x)>0.001 || fabs(origin.y-expected.y)>0.001) abort();
                checks++;
                [rows addObject:@{@"mode":mode,@"frameOrigin":@[@(w.frame.origin.x),@(w.frame.origin.y)],
                    @"pointer":@[@(screen.x),@(screen.y)],@"eventLocation":@[@(e.locationInWindow.x),@(e.locationInWindow.y)],
                    @"eventWindow":@(e.windowNumber),@"dragOffset":@[@(offset.x),@(offset.y)],
                    @"impliedOrigin":@[@(origin.x),@(origin.y)]}];
            }
          }
        }
        NSData *data=[NSJSONSerialization dataWithJSONObject:rows options:NSJSONWritingPrettyPrinted error:nil];
        NSString *output = argc > 1 ? @(argv[1]) : @"/tmp/askhuman-drag-offset-results.json";
        if (![data writeToFile:output atomically:YES]) abort();
        printf("PASS: %lu native coordinate checks; full results: %s\n", (unsigned long)checks, output.UTF8String);
    }
}

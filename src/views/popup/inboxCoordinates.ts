export interface CanvasPosition { canvas?: { left: number } | null; sidebarWidth: number }
// Wry/macOS reports draggingLocation in window points, even though the IPC type says physical.
// A fixed canvas starts outside the window, so native drag locations need one origin translation.
export function inboxDropPoint(x: number, y: number, layout: CanvasPosition): [number, number] {
  const left = layout.sidebarWidth + (layout.sidebarWidth > 0 ? 6 : 0);
  return [x + (layout.canvas?.left ?? left) - left, y];
}

export interface ComposerDockGeometry {
  homeTop: number;
  homeBottom: number;
  viewportTop: number;
  viewportBottom: number;
  viewportBottomAfterUndock: number;
}

export interface ComposerDockThresholds {
  dockGap: number;
  returnGap: number;
}

export const DEFAULT_COMPOSER_DOCK_THRESHOLDS: ComposerDockThresholds = {
  dockGap: 0,
  returnGap: 10,
};

/** A focused editor owns user actions even when passive scrolling changes the viewport card. */
export function resolveActionQuestionIndex(
  viewportQuestion: number,
  focusedQuestion: number | null
): number {
  return focusedQuestion ?? viewportQuestion;
}

export function cmdEnterQuestionIndex(
  currentQuestion: number,
  focusedQuestion: number | null
): number {
  return resolveActionQuestionIndex(currentQuestion, focusedQuestion);
}

export function shouldRevealQuestionBeforeCmdEnter(
  questionIndex: number,
  focusedQuestion: number | null,
  dockedQuestion: number | null,
  cardOffScreen: boolean
): boolean {
  const focusedEditorIsDocked =
    focusedQuestion === questionIndex && dockedQuestion === questionIndex;
  return cardOffScreen && !focusedEditorIsDocked;
}

/** Only a real scroll event may hand the current-question pointer back to scroll-spy. */
export function shouldApplyScrollSpy(
  scrollEventPending: boolean,
  verticalMode: boolean,
  nowMs: number,
  activeLockUntilMs: number,
): boolean {
  return scrollEventPending && verticalMode && nowMs >= activeLockUntilMs;
}

export function shouldDeactivateOffscreenComposer(
  focusedQuestion: number | null,
  dockedQuestion: number | null,
  cardOffScreen: boolean
): boolean {
  return (
    focusedQuestion !== null &&
    dockedQuestion !== focusedQuestion &&
    cardOffScreen
  );
}

export function canComposerDock(
  focused: boolean,
  manuallyActivated: boolean,
  seenFullyInline: boolean,
  scrolledUpAfterActivation: boolean
): boolean {
  return (
    focused &&
    scrolledUpAfterActivation &&
    (manuallyActivated || seenFullyInline)
  );
}

export function composerHomeVisibleRatio(geometry: ComposerDockGeometry): number {
  const height = geometry.homeBottom - geometry.homeTop;
  if (height <= 0) return 0;
  const visibleHeight = Math.max(
    0,
    Math.min(geometry.homeBottom, geometry.viewportBottom) -
      Math.max(geometry.homeTop, geometry.viewportTop)
  );
  return Math.min(1, visibleHeight / height);
}

export function isComposerHomeFullyVisible(
  geometry: ComposerDockGeometry
): boolean {
  return (
    geometry.homeTop >= geometry.viewportTop &&
    geometry.homeBottom <= geometry.viewportBottom
  );
}

export function resolveComposerDocked(
  currentlyDocked: boolean,
  ownerCanDock: boolean,
  geometry: ComposerDockGeometry,
  thresholds: ComposerDockThresholds = DEFAULT_COMPOSER_DOCK_THRESHOLDS
): boolean {
  if (!ownerCanDock) return false;

  // Bottom docking only covers looking back at content above the composer. If the source has
  // moved above the viewport, keep the editor in its normal document position.
  if (geometry.homeTop < geometry.viewportTop) return false;

  if (!currentlyDocked) {
    return geometry.homeBottom > geometry.viewportBottom - thresholds.dockGap;
  }

  const returnedInsideViewport =
    geometry.homeBottom <=
    geometry.viewportBottomAfterUndock - thresholds.returnGap;
  return !returnedInsideViewport;
}

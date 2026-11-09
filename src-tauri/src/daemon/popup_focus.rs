//! Pure daemon-side arbitration for concurrently visible popup helpers.
//!
//! Each popup lives in a separate process, so focus ownership must be decided by the daemon.
//! This module intentionally has no Tauri or IPC dependencies: callers apply the returned effects
//! after releasing their outer locks.

use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadyMetadata {
    /// Global AppKit window number on macOS. Other platforms leave it empty.
    pub window_number: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupEffect {
    PresentForeground {
        request_id: String,
    },
    PresentBackground {
        request_id: String,
        cascade_index: u32,
        behind_window_number: Option<i64>,
    },
    Focus {
        request_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum SurfacePhase {
    Reserved,
    Ready,
    Presented,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PopupSurface {
    seq: u64,
    phase: SurfacePhase,
    window_number: Option<i64>,
    terminal: bool,
}

impl PopupSurface {
    fn new(seq: u64) -> Self {
        Self {
            seq,
            phase: SurfacePhase::Reserved,
            window_number: None,
            terminal: false,
        }
    }
}

/// Single authority for automatic popup focus and background cascade order.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PopupFocusArbiter {
    owner: Option<String>,
    waiting: VecDeque<String>,
    entries: HashMap<String, PopupSurface>,
}

impl PopupFocusArbiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.owner = None;
        self.waiting.clear();
        self.entries.clear();
    }

    pub fn retain_live(&mut self, live: &std::collections::HashSet<String>) {
        self.entries.retain(|id, _| live.contains(id));
        self.waiting.retain(|id| self.entries.contains_key(id));
        self.owner = self.owner.take().filter(|id| self.entries.contains_key(id));
        if self.owner.is_none() {
            self.owner = self
                .entries
                .iter()
                .min_by_key(|(_, surface)| surface.seq)
                .map(|(id, _)| id.clone());
        }
        self.waiting.retain(|id| Some(id) != self.owner.as_ref());
    }

    /// Reserve a popup slot before dispatching its helper.
    pub fn reserve(&mut self, request_id: impl Into<String>, seq: u64) -> Vec<PopupEffect> {
        let request_id = request_id.into();
        if self.entries.contains_key(&request_id) {
            return Vec::new();
        }
        self.entries
            .insert(request_id.clone(), PopupSurface::new(seq));

        match self.owner.as_ref() {
            None => self.owner = Some(request_id),
            Some(owner_id) => {
                let owner = self
                    .entries
                    .get(owner_id)
                    .expect("popup owner must exist in the arbiter");
                // A lower sequence may reserve slightly later when concurrent submit tasks race.
                // It may replace an owner only while that owner is still fully hidden.
                if owner.phase == SurfacePhase::Reserved && seq < owner.seq {
                    let old_owner = self.owner.replace(request_id).unwrap();
                    self.insert_waiting_sorted(old_owner);
                } else {
                    self.insert_waiting_sorted(request_id);
                }
            }
        }
        Vec::new()
    }

    /// Mark a hidden window ready for the daemon to present.
    pub fn ready(&mut self, request_id: &str, metadata: ReadyMetadata) -> Vec<PopupEffect> {
        let Some(surface) = self.entries.get_mut(request_id) else {
            return Vec::new();
        };
        if surface.terminal || surface.phase != SurfacePhase::Reserved {
            return Vec::new();
        }
        surface.phase = SurfacePhase::Ready;
        surface.window_number = metadata.window_number;
        self.reconcile_presentations()
    }

    /// Transfer ownership after a real native focus event. The target already owns OS focus, so
    /// no redundant Focus effect is emitted.
    pub fn claim(&mut self, request_id: &str) -> Vec<PopupEffect> {
        self.claim_inner(request_id, false)
    }

    /// Transfer ownership after an explicit tray action and focus the target if it is visible.
    pub fn claim_and_focus(&mut self, request_id: &str) -> Vec<PopupEffect> {
        self.claim_inner(request_id, true)
    }

    /// Mark the request terminal. Visible windows retain ownership until dismissal so the next
    /// popup cannot fight a closing window for focus.
    pub fn terminal(&mut self, request_id: &str) -> Vec<PopupEffect> {
        let Some(surface) = self.entries.get_mut(request_id) else {
            return Vec::new();
        };
        if surface.terminal {
            return Vec::new();
        }
        surface.terminal = true;
        if surface.phase == SurfacePhase::Presented {
            Vec::new()
        } else {
            self.remove_surface(request_id)
        }
    }

    /// The native window is gone. This always releases its place, even if request finalization is
    /// still racing with the GUI event.
    pub fn dismissed(&mut self, request_id: &str) -> Vec<PopupEffect> {
        self.remove_surface(request_id)
    }

    /// A failed dispatch or dead helper cannot retain ownership. Both paths have identical focus
    /// consequences; separate methods keep call sites expressive.
    pub fn dispatch_failed(&mut self, request_id: &str) -> Vec<PopupEffect> {
        self.remove_surface(request_id)
    }

    pub fn disconnected(&mut self, request_id: &str) -> Vec<PopupEffect> {
        self.remove_surface(request_id)
    }

    fn claim_inner(&mut self, request_id: &str, ensure_focus: bool) -> Vec<PopupEffect> {
        let Some(target) = self.entries.get(request_id) else {
            return Vec::new();
        };
        if target.terminal {
            return Vec::new();
        }

        if self.owner.as_deref() != Some(request_id) {
            self.waiting.retain(|id| id != request_id);
            if let Some(old_owner) = self.owner.replace(request_id.to_string()) {
                if self
                    .entries
                    .get(&old_owner)
                    .is_some_and(|surface| !surface.terminal)
                {
                    self.waiting.push_front(old_owner);
                }
            }
        }

        let mut effects = self.reconcile_presentations();
        if ensure_focus
            && self
                .entries
                .get(request_id)
                .is_some_and(|surface| surface.phase == SurfacePhase::Presented)
            && !effects.iter().any(|effect| {
                matches!(
                    effect,
                    PopupEffect::PresentForeground { request_id: id } if id == request_id
                )
            })
        {
            effects.push(PopupEffect::Focus {
                request_id: request_id.to_string(),
            });
        }
        effects
    }

    fn remove_surface(&mut self, request_id: &str) -> Vec<PopupEffect> {
        if self.entries.remove(request_id).is_none() {
            return Vec::new();
        }
        self.waiting.retain(|id| id != request_id);
        if self.owner.as_deref() != Some(request_id) {
            return self.reconcile_presentations();
        }

        self.owner = None;
        while let Some(candidate) = self.waiting.pop_front() {
            if self
                .entries
                .get(&candidate)
                .is_some_and(|surface| !surface.terminal)
            {
                self.owner = Some(candidate);
                break;
            }
            self.entries.remove(&candidate);
        }

        let mut effects = self.reconcile_presentations();
        if let Some(owner) = self.owner.as_ref() {
            let already_presented_now = effects.iter().any(|effect| {
                matches!(
                    effect,
                    PopupEffect::PresentForeground { request_id } if request_id == owner
                )
            });
            if !already_presented_now
                && self
                    .entries
                    .get(owner)
                    .is_some_and(|surface| surface.phase == SurfacePhase::Presented)
            {
                effects.insert(
                    0,
                    PopupEffect::Focus {
                        request_id: owner.clone(),
                    },
                );
            }
        }
        effects
    }

    fn reconcile_presentations(&mut self) -> Vec<PopupEffect> {
        let mut effects = Vec::new();
        let Some(owner_id) = self.owner.clone() else {
            return effects;
        };

        let owner_presented = match self.entries.get_mut(&owner_id) {
            Some(owner) if !owner.terminal && owner.phase == SurfacePhase::Ready => {
                owner.phase = SurfacePhase::Presented;
                effects.push(PopupEffect::PresentForeground {
                    request_id: owner_id.clone(),
                });
                true
            }
            Some(owner) => owner.phase == SurfacePhase::Presented,
            None => false,
        };
        if !owner_presented {
            return effects;
        }

        let mut predecessor = self
            .entries
            .get(&owner_id)
            .and_then(|surface| surface.window_number);
        let waiting: Vec<String> = self.waiting.iter().cloned().collect();
        let mut cascade_index = 1u32;
        for request_id in waiting {
            let Some(surface) = self.entries.get_mut(&request_id) else {
                continue;
            };
            if surface.terminal {
                continue;
            }
            match surface.phase {
                SurfacePhase::Reserved => break,
                SurfacePhase::Ready => {
                    surface.phase = SurfacePhase::Presented;
                    effects.push(PopupEffect::PresentBackground {
                        request_id: request_id.clone(),
                        cascade_index,
                        behind_window_number: predecessor,
                    });
                }
                SurfacePhase::Presented => {}
            }
            predecessor = surface.window_number.or(predecessor);
            cascade_index = cascade_index.saturating_add(1);
        }
        effects
    }

    fn insert_waiting_sorted(&mut self, request_id: String) {
        let seq = self
            .entries
            .get(&request_id)
            .expect("waiting popup must exist")
            .seq;
        let position = self.waiting.iter().position(|candidate| {
            self.entries
                .get(candidate)
                .is_some_and(|surface| surface.seq > seq)
        });
        match position {
            Some(index) => self.waiting.insert(index, request_id),
            None => self.waiting.push_back(request_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(window_number: i64) -> ReadyMetadata {
        ReadyMetadata {
            window_number: Some(window_number),
        }
    }

    #[test]
    fn a_single_ready_popup_is_presented_in_front() {
        let mut arbiter = PopupFocusArbiter::new();
        arbiter.reserve("a", 1);
        assert_eq!(
            arbiter.ready("a", ready(11)),
            vec![PopupEffect::PresentForeground {
                request_id: "a".into()
            }]
        );
    }

    #[test]
    fn later_ready_popup_waits_for_its_predecessor() {
        let mut arbiter = PopupFocusArbiter::new();
        arbiter.reserve("a", 1);
        arbiter.reserve("b", 2);
        assert!(arbiter.ready("b", ready(22)).is_empty());
        assert_eq!(
            arbiter.ready("a", ready(11)),
            vec![
                PopupEffect::PresentForeground {
                    request_id: "a".into()
                },
                PopupEffect::PresentBackground {
                    request_id: "b".into(),
                    cascade_index: 1,
                    behind_window_number: Some(11),
                },
            ]
        );
    }

    #[test]
    fn three_ready_popups_form_a_background_chain() {
        let mut arbiter = PopupFocusArbiter::new();
        arbiter.reserve("a", 1);
        arbiter.reserve("b", 2);
        arbiter.reserve("c", 3);
        assert_eq!(
            arbiter.ready("a", ready(11)),
            vec![PopupEffect::PresentForeground {
                request_id: "a".into()
            }]
        );
        assert_eq!(
            arbiter.ready("b", ready(22)),
            vec![PopupEffect::PresentBackground {
                request_id: "b".into(),
                cascade_index: 1,
                behind_window_number: Some(11),
            }]
        );
        assert_eq!(
            arbiter.ready("c", ready(33)),
            vec![PopupEffect::PresentBackground {
                request_id: "c".into(),
                cascade_index: 2,
                behind_window_number: Some(22),
            }]
        );
    }

    #[test]
    fn terminal_owner_waits_for_dismissal_before_handoff() {
        let mut arbiter = PopupFocusArbiter::new();
        arbiter.reserve("a", 1);
        arbiter.reserve("b", 2);
        arbiter.ready("a", ready(11));
        arbiter.ready("b", ready(22));
        assert!(arbiter.terminal("a").is_empty());
        assert_eq!(
            arbiter.dismissed("a"),
            vec![PopupEffect::Focus {
                request_id: "b".into()
            }]
        );
    }

    #[test]
    fn failed_hidden_owner_promotes_ready_waiter() {
        let mut arbiter = PopupFocusArbiter::new();
        arbiter.reserve("a", 1);
        arbiter.reserve("b", 2);
        assert!(arbiter.ready("b", ready(22)).is_empty());
        assert_eq!(
            arbiter.dispatch_failed("a"),
            vec![PopupEffect::PresentForeground {
                request_id: "b".into()
            }]
        );
    }

    #[test]
    fn failed_waiter_does_not_disturb_the_rest_of_the_queue() {
        let mut arbiter = PopupFocusArbiter::new();
        arbiter.reserve("a", 1);
        arbiter.reserve("b", 2);
        arbiter.reserve("c", 3);
        arbiter.ready("a", ready(11));
        arbiter.ready("b", ready(22));
        assert!(arbiter.dispatch_failed("b").is_empty());
        assert_eq!(
            arbiter.ready("c", ready(33)),
            vec![PopupEffect::PresentBackground {
                request_id: "c".into(),
                cascade_index: 1,
                behind_window_number: Some(11),
            }]
        );
    }

    #[test]
    fn native_focus_claim_moves_the_old_owner_to_the_front_of_waiting() {
        let mut arbiter = PopupFocusArbiter::new();
        arbiter.reserve("a", 1);
        arbiter.reserve("b", 2);
        arbiter.reserve("c", 3);
        arbiter.ready("a", ready(11));
        arbiter.ready("b", ready(22));
        arbiter.ready("c", ready(33));

        assert!(arbiter.claim("c").is_empty());
        assert!(arbiter.terminal("c").is_empty());
        assert_eq!(
            arbiter.dismissed("c"),
            vec![PopupEffect::Focus {
                request_id: "a".into()
            }]
        );
    }

    #[test]
    fn tray_claim_focuses_a_presented_waiter() {
        let mut arbiter = PopupFocusArbiter::new();
        arbiter.reserve("a", 1);
        arbiter.reserve("b", 2);
        arbiter.ready("a", ready(11));
        arbiter.ready("b", ready(22));
        assert_eq!(
            arbiter.claim_and_focus("b"),
            vec![PopupEffect::Focus {
                request_id: "b".into()
            }]
        );
    }

    #[test]
    fn tray_claim_before_ready_presents_target_in_front_when_ready() {
        let mut arbiter = PopupFocusArbiter::new();
        arbiter.reserve("a", 1);
        arbiter.reserve("b", 2);
        arbiter.ready("a", ready(11));
        assert!(arbiter.claim_and_focus("b").is_empty());
        assert_eq!(
            arbiter.ready("b", ready(22)),
            vec![PopupEffect::PresentForeground {
                request_id: "b".into()
            }]
        );
    }

    #[test]
    fn duplicate_and_late_events_are_idempotent() {
        let mut arbiter = PopupFocusArbiter::new();
        arbiter.reserve("a", 1);
        arbiter.reserve("a", 1);
        arbiter.ready("a", ready(11));
        assert!(arbiter.ready("a", ready(11)).is_empty());
        assert!(arbiter.terminal("a").is_empty());
        assert!(arbiter.terminal("a").is_empty());
        assert!(arbiter.dismissed("a").is_empty());
        assert!(arbiter.dismissed("a").is_empty());
        assert!(arbiter.ready("a", ready(11)).is_empty());
    }

    #[test]
    fn lower_sequence_can_replace_only_a_hidden_owner() {
        let mut arbiter = PopupFocusArbiter::new();
        arbiter.reserve("b", 2);
        arbiter.reserve("a", 1);
        assert_eq!(
            arbiter.ready("a", ready(11)),
            vec![PopupEffect::PresentForeground {
                request_id: "a".into()
            }]
        );

        let mut already_visible = PopupFocusArbiter::new();
        already_visible.reserve("b", 2);
        already_visible.ready("b", ready(22));
        already_visible.reserve("a", 1);
        assert_eq!(
            already_visible.ready("a", ready(11)),
            vec![PopupEffect::PresentBackground {
                request_id: "a".into(),
                cascade_index: 1,
                behind_window_number: Some(22),
            }]
        );
    }
    #[test]
    fn restored_windows_keep_focus_order_without_being_presented_again() {
        let mut focus = PopupFocusArbiter::new();
        focus.reserve("original", 1);
        focus.ready(
            "original",
            ReadyMetadata {
                window_number: Some(10),
            },
        );
        focus.reserve("background", 2);
        focus.ready(
            "background",
            ReadyMetadata {
                window_number: Some(11),
            },
        );
        let bytes = serde_json::to_vec(&focus).unwrap();
        let mut restored: PopupFocusArbiter = serde_json::from_slice(&bytes).unwrap();
        restored.retain_live(
            &["original".into(), "background".into()]
                .into_iter()
                .collect(),
        );
        assert!(restored
            .ready(
                "original",
                ReadyMetadata {
                    window_number: Some(10)
                }
            )
            .is_empty());
        restored.reserve("new", 3);
        assert!(
            matches!(restored.ready("new", ReadyMetadata { window_number: Some(12) }).as_slice(), [PopupEffect::PresentBackground { request_id, .. }] if request_id == "new")
        );
    }
}

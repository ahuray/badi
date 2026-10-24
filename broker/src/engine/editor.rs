//! Editor focus reports: an editor with its own Badi integration (Neovim)
//! running in a terminal says when it holds that terminal's focus. While any
//! editor holds focus, the input method's native path (desktop-application
//! targets, such as a terminal's typed line) is withheld, so the keys typed
//! into the editor get one suggester, the editor's own (ADR 0009).
//!
//! A report never grants authority. Each one is bound to the connection
//! that sent it, so an editor that exits or loses its bridge takes its hold
//! with it.

use super::{Broker, BrokerState};
use crate::protocol::{EditorStatusPayload, ReasonCode, TargetDescriptor, TargetKind};

impl Broker {
    /// Records one editor connection's focus. Starting or ending the hold
    /// advances the authority epoch: native sessions lose their context and
    /// every adapter resolves its policy afresh.
    pub async fn report_editor_focus(
        &self,
        connection_id: &str,
        focused: bool,
    ) -> EditorStatusPayload {
        let mut state = self.inner.state.lock().await;
        let was_held = state.editor_focus_held();
        if focused {
            state.editor_focus.insert(connection_id.to_owned());
        } else {
            state.editor_focus.remove(connection_id);
        }
        let held = state.editor_focus_held();
        if held != was_held {
            state.advance_authority_epoch();
            if held {
                for session in state.sessions.values_mut() {
                    if withheld_by_editor(&session.target.target) {
                        session.revoke_context_authority(
                            &self.inner.metrics,
                            ReasonCode::FocusChanged,
                        );
                    }
                }
            }
            self.publish_authority_change(state);
        }
        EditorStatusPayload {
            native_withheld: held,
        }
    }

    /// Drops a closed editor connection's focus and any hold it placed.
    pub async fn forget_editor_focus(&self, connection_id: &str) {
        let _ = self.report_editor_focus(connection_id, false).await;
    }
}

impl BrokerState {
    pub(super) fn editor_focus_held(&self) -> bool {
        !self.editor_focus.is_empty()
    }

    /// The target belongs to the native path an editor's focus withholds.
    pub(super) fn editor_withholds(&self, target: &TargetDescriptor) -> bool {
        self.editor_focus_held() && withheld_by_editor(target)
    }
}

/// Only the input method's desktop-application targets: the focused window
/// is the editor's terminal, and browsers, editors with their own adapters
/// and Writer keep their policy.
pub(super) fn withheld_by_editor(target: &TargetDescriptor) -> bool {
    target.kind == TargetKind::DesktopApplication
}

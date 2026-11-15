------------------------------ MODULE Withhold ------------------------------
(***************************************************************************)
(* Pause, snooze and screen-share withholding for one broker.             *)
(*                                                                         *)
(* The promise: a suggestion stays withdrawn while a settings pause, a     *)
(* runtime pause, a desktop hold (an inactive or sleeping session, or a    *)
(* screen share in hold mode, including the grace after its helper         *)
(* leaves) or a snooze of that app or site is in force. Ending one of      *)
(* those does not lift the others. A snooze records nothing while the      *)
(* broker is effectively paused, and nothing when the focused fields       *)
(* belong to different apps or sites. A snooze of one identity does not    *)
(* withdraw the other.                                                     *)
(*                                                                         *)
(* Bounds: two sessions (sa = mail, sb = docs), two helpers, one grace     *)
(* phase, snooze on or off per identity. Sleeping and inactive both       *)
(* withhold and are one helper state. The control plane is healthy and    *)
(* not mid-mutation; a settings replacement is one step. Editor focus,    *)
(* battery scheduling, commit grants and wall-clock durations are out.    *)
(***************************************************************************)

EXTENDS FiniteSets

Sessions == {"sa", "sb"}
Helpers == {"h1", "h2"}
Identities == {"mail", "docs"}
Modes == {"hold", "suggest"}
\* absent: no report. active/shared: DesktopSession::Active.
\* inactive/inactive_shared: DesktopSession::holds (inactive or sleeping).
HelperStates == {"absent", "active", "shared", "inactive", "inactive_shared"}

\* SessionState phase. closed: not in BrokerState.sessions.
\* open: session without context. focused: context.field.focused.
Phases == {"closed", "open", "focused"}

VARIABLES
  user_paused,      \* BrokerState.paused
  settings_paused,  \* SettingsV2.paused
  screen_mode,      \* SettingsV2.screen_share
  helper,           \* connection id -> last DesktopStatePayload, or absent
  share_grace,      \* BrokerState.share_grace is Some
  snooze,           \* identity -> still inside BrokerState.snoozes
  phase,            \* session -> Phases
  shown             \* session -> admit_generation admitted and not revoked

vars == <<user_paused, settings_paused, screen_mode, helper, share_grace, snooze, phase, shown>>

identity == [s \in Sessions |-> IF s = "sa" THEN "mail" ELSE "docs"]

HelperShared(state) == state \in {"shared", "inactive_shared"}
HelperInactive(state) == state \in {"inactive", "inactive_shared"}

\* BrokerState::screen_share
ScreenShareOf(helpers, grace, mode) ==
  IF /\ ~grace
     /\ ~\E h \in Helpers: HelperShared(helpers[h])
  THEN "none"
  ELSE IF mode = "hold" THEN "held" ELSE "suggesting"

\* BrokerState::desktop_hold
DesktopHoldOf(helpers, grace, mode) ==
  \/ \E h \in Helpers: HelperInactive(helpers[h])
  \/ ScreenShareOf(helpers, grace, mode) = "held"

\* BrokerState::runtime_paused, with a healthy control plane.
RuntimePaused ==
  \/ user_paused
  \/ DesktopHoldOf(helper, share_grace, screen_mode)

\* BrokerState::effective_paused
EffectivePaused == RuntimePaused \/ settings_paused

FocusedIds == {identity[s] : s \in {t \in Sessions : phase[t] = "focused"}}

\* BrokerState::session_with_data_access for a live session, plus
\* settings_allow_data / PolicyResolution::allows_suggestion (paused denies).
DataAccess(s) ==
  /\ phase[s] # "closed"
  /\ ~snooze[identity[s]]
  /\ ~RuntimePaused
  /\ ~settings_paused

\* open_session's policy_status gate: suggestions_allowed.
SuggestionsAllowed(id) ==
  /\ ~RuntimePaused
  /\ ~settings_paused
  /\ ~snooze[id]

TypeOK ==
  /\ user_paused \in BOOLEAN
  /\ settings_paused \in BOOLEAN
  /\ screen_mode \in Modes
  /\ helper \in [Helpers -> HelperStates]
  /\ share_grace \in BOOLEAN
  /\ snooze \in [Identities -> BOOLEAN]
  /\ phase \in [Sessions -> Phases]
  /\ shown \in [Sessions -> BOOLEAN]

Init ==
  /\ user_paused = FALSE
  /\ settings_paused = FALSE
  /\ screen_mode = "hold"
  /\ helper = [h \in Helpers |-> "absent"]
  /\ share_grace = FALSE
  /\ snooze = [id \in Identities |-> FALSE]
  /\ phase = [s \in Sessions |-> "closed"]
  /\ shown = [s \in Sessions |-> FALSE]

\* Context revoked, session kept. SessionState::revoke_context_authority.
Revoked ==
  /\ phase' = [s \in Sessions |-> IF phase[s] = "focused" THEN "open" ELSE phase[s]]
  /\ shown' = [s \in Sessions |-> FALSE]

\* Broker::finish_desktop_transition: revoke only when the hold begins.
HoldStep(before, after) ==
  IF after /\ ~before THEN Revoked ELSE UNCHANGED <<phase, shown>>

\* Broker::set_paused -> BrokerState::set_runtime_pause
SetUserPause(p) ==
  /\ user_paused # p
  /\ user_paused' = p
  /\ UNCHANGED <<settings_paused, screen_mode, helper, share_grace, snooze>>
  /\ IF p \/ DesktopHoldOf(helper, share_grace, screen_mode) \/ settings_paused
     THEN Revoked
     ELSE UNCHANGED <<phase, shown>>

\* Broker::replace_settings -> begin_control_plane_mutation
\* -> install_settings_authority -> BrokerState::close_all_sessions
\* and BrokerState::install_settings. One step: the mutation gap is
\* fail-closed (sessions already closed, runtime paused) and admits nothing.
ReplaceSettings(paused, mode) ==
  /\ (settings_paused # paused \/ screen_mode # mode)
  /\ settings_paused' = paused
  /\ screen_mode' = mode
  /\ phase' = [s \in Sessions |-> "closed"]
  /\ shown' = [s \in Sessions |-> FALSE]
  /\ UNCHANGED <<user_paused, helper, share_grace, snooze>>

\* Broker::report_desktop_state. A fresh report ends any grace.
Report(h, kind) ==
  /\ kind # "absent"
  /\ LET nextHelper == [helper EXCEPT ![h] = kind]
         before == DesktopHoldOf(helper, share_grace, screen_mode)
         after == DesktopHoldOf(nextHelper, FALSE, screen_mode)
     IN /\ helper' = nextHelper
        /\ share_grace' = FALSE
        /\ HoldStep(before, after)
        /\ UNCHANGED <<user_paused, settings_paused, screen_mode, snooze>>

\* Broker::forget_desktop_state. A helper that last reported a share
\* leaves share_grace set. A missing connection does nothing.
Forget(h) ==
  /\ helper[h] # "absent"
  /\ LET wasShared == HelperShared(helper[h])
         nextHelper == [helper EXCEPT ![h] = "absent"]
         nextGrace == IF wasShared THEN TRUE ELSE share_grace
         before == DesktopHoldOf(helper, share_grace, screen_mode)
         after == DesktopHoldOf(nextHelper, nextGrace, screen_mode)
     IN /\ helper' = nextHelper
        /\ share_grace' = nextGrace
        /\ HoldStep(before, after)
        /\ UNCHANGED <<user_paused, settings_paused, screen_mode, snooze>>

\* Broker::end_share_grace, for the one grace phase this model has.
EndShareGrace ==
  /\ share_grace
  /\ LET before == DesktopHoldOf(helper, TRUE, screen_mode)
         after == DesktopHoldOf(helper, FALSE, screen_mode)
     IN /\ share_grace' = FALSE
        /\ HoldStep(before, after)
        /\ UNCHANGED <<helper, user_paused, settings_paused, screen_mode, snooze>>

\* Broker::open_session
Open(s) ==
  /\ phase[s] = "closed"
  /\ SuggestionsAllowed(identity[s])
  /\ phase' = [phase EXCEPT ![s] = "open"]
  /\ UNCHANGED <<shown, user_paused, settings_paused, screen_mode, helper, share_grace, snooze>>

\* Broker::close_session
Close(s) ==
  /\ phase[s] # "closed"
  /\ phase' = [phase EXCEPT ![s] = "closed"]
  /\ shown' = [shown EXCEPT ![s] = FALSE]
  /\ UNCHANGED <<user_paused, settings_paused, screen_mode, helper, share_grace, snooze>>

\* Broker::update_context with field.focused, when session_with_data_access allows it.
Focus(s) ==
  /\ phase[s] = "open"
  /\ DataAccess(s)
  /\ phase' = [phase EXCEPT ![s] = "focused"]
  /\ UNCHANGED <<shown, user_paused, settings_paused, screen_mode, helper, share_grace, snooze>>

\* Broker::request_suggestion -> admit_generation.
\* The captured effective_paused and session_with_data_access both have to pass.
Admit(s) ==
  /\ phase[s] = "focused"
  /\ ~EffectivePaused
  /\ DataAccess(s)
  /\ shown' = [shown EXCEPT ![s] = TRUE]
  /\ UNCHANGED <<phase, user_paused, settings_paused, screen_mode, helper, share_grace, snooze>>

\* Broker::snooze_focused -> BrokerState::focused_subject.
\* Refusals (paused, no context, ambiguous_session) change no variable,
\* so they are not steps. A started snooze revokes only that identity.
Snooze ==
  /\ ~EffectivePaused
  /\ Cardinality(FocusedIds) = 1
  /\ \E id \in FocusedIds:
       /\ snooze' = [snooze EXCEPT ![id] = TRUE]
       /\ phase' = [s \in Sessions |->
            IF phase[s] = "focused" /\ identity[s] = id THEN "open" ELSE phase[s]]
       /\ shown' = [s \in Sessions |-> IF identity[s] = id THEN FALSE ELSE shown[s]]
       /\ UNCHANGED <<user_paused, settings_paused, screen_mode, helper, share_grace>>

\* Broker::end_snoozes. Does not restore context and does not clear a pause or a hold.
EndSnoozes ==
  /\ \E id \in Identities: snooze[id]
  /\ snooze' = [id \in Identities |-> FALSE]
  /\ UNCHANGED <<user_paused, settings_paused, screen_mode, helper, share_grace, phase, shown>>

\* Broker::expire_snoozes, one identity at a time (distinct deadlines).
Expire(id) ==
  /\ snooze[id]
  /\ snooze' = [snooze EXCEPT ![id] = FALSE]
  /\ UNCHANGED <<user_paused, settings_paused, screen_mode, helper, share_grace, phase, shown>>

Next ==
  \/ \E p \in BOOLEAN: SetUserPause(p)
  \/ \E paused \in BOOLEAN, mode \in Modes: ReplaceSettings(paused, mode)
  \/ \E h \in Helpers, kind \in HelperStates: Report(h, kind)
  \/ \E h \in Helpers: Forget(h)
  \/ EndShareGrace
  \/ \E s \in Sessions: Open(s)
  \/ \E s \in Sessions: Close(s)
  \/ \E s \in Sessions: Focus(s)
  \/ \E s \in Sessions: Admit(s)
  \/ Snooze
  \/ EndSnoozes
  \/ \E id \in Identities: Expire(id)

Spec == Init /\ [][Next]_vars

\* A shown suggestion has a focused field and no withhold in force.
SuggestionsStayWithheld ==
  \A s \in Sessions:
    shown[s] =>
      /\ phase[s] = "focused"
      /\ ~user_paused
      /\ ~settings_paused
      /\ ~DesktopHoldOf(helper, share_grace, screen_mode)
      /\ ~snooze[identity[s]]

\* Ending one withhold does not clear the others.
\* A settings write may change the pause flag and the screen-share mode together.
OneWithholdDoesNotLiftAnother ==
  [][
    /\ (settings_paused /\ ~settings_paused') =>
         UNCHANGED <<user_paused, snooze, helper, share_grace>>
    /\ (user_paused /\ ~user_paused') =>
         UNCHANGED <<settings_paused, snooze, helper, share_grace, screen_mode>>
    /\ (\E id \in Identities: snooze[id] /\ ~snooze'[id]) =>
         UNCHANGED <<user_paused, settings_paused, helper, share_grace, screen_mode>>
    /\ DesktopHoldOf(helper, share_grace, screen_mode)
         /\ ~DesktopHoldOf(helper', share_grace', screen_mode')
         /\ screen_mode' = screen_mode =>
         UNCHANGED <<user_paused, settings_paused, snooze>>
  ]_vars

\* A snooze starts only for the one focused identity, and only while not
\* effectively paused. It does not withdraw the other identity.
SnoozeStartsOnlyForOneClearSubject ==
  [][
    \A id \in Identities:
      (~snooze[id] /\ snooze'[id]) =>
        /\ ~EffectivePaused
        /\ FocusedIds = {id}
        /\ \A other \in Identities: other # id => snooze'[other] = snooze[other]
        /\ \A s \in Sessions:
             identity[s] # id => shown'[s] = shown[s] /\ phase'[s] = phase[s]
  ]_vars

=============================================================================

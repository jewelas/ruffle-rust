//! Core event structure

use crate::avm2::object::{Object, TObject};
use crate::avm2::string::AvmString;
use gc_arena::Collect;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

/// Which phase of event dispatch is currently occurring.
#[derive(Copy, Clone, Collect, Debug, PartialEq, Eq)]
#[collect(require_static)]
pub enum EventPhase {
    /// The event has yet to be fired on the target and is descending the
    /// ancestors of the event target.
    Capturing,

    /// The event is currently firing on the target.
    AtTarget,

    /// The event has already fired on the target and is ascending the
    /// ancestors of the event target.
    Bubbling,
}

impl Into<u32> for EventPhase {
    fn into(self) -> u32 {
        match self {
            Self::Capturing => 1,
            Self::AtTarget => 2,
            Self::Bubbling => 3,
        }
    }
}

/// How this event is allowed to propagate.
#[derive(Copy, Clone, Collect, Debug, PartialEq, Eq)]
#[collect(require_static)]
pub enum PropagationMode {
    /// Propagate events normally.
    AllowPropagation,

    /// Stop capturing or bubbling events.
    StopPropagation,

    /// Stop running event handlers altogether.
    StopImmediatePropagation,
}

/// Represents data fields of an event that can be fired on an object that
/// implements `IEventDispatcher`.
#[derive(Clone, Collect, Debug)]
#[collect(no_drop)]
pub struct Event<'gc> {
    /// Whether or not the event "bubbles" - fires on it's parents after it
    /// fires on the child.
    bubbles: bool,

    /// Whether or not the event has a default response that an event handler
    /// can request to not occur.
    cancelable: bool,

    /// Whether or not the event's default response has been cancelled.
    cancelled: bool,

    /// Whether or not event propagation has stopped.
    propagation: PropagationMode,

    /// The object currently having it's event handlers invoked.
    current_target: Option<Object<'gc>>,

    /// The current event phase.
    event_phase: EventPhase,

    /// The object this event was dispatched on.
    target: Option<Object<'gc>>,

    /// The name of the event being triggered.
    event_type: AvmString<'gc>,
}

impl<'gc> Event<'gc> {
    /// Construct a new event of a given type.
    pub fn new<S>(event_type: S) -> Self
    where
        S: Into<AvmString<'gc>>,
    {
        Event {
            bubbles: false,
            cancelable: false,
            cancelled: false,
            propagation: PropagationMode::AllowPropagation,
            current_target: None,
            event_phase: EventPhase::Bubbling,
            target: None,
            event_type: event_type.into(),
        }
    }

    pub fn event_type(&self) -> AvmString<'gc> {
        self.event_type
    }

    pub fn set_event_type<S>(&mut self, event_type: S)
    where
        S: Into<AvmString<'gc>>,
    {
        self.event_type = event_type.into();
    }

    pub fn is_bubbling(&self) -> bool {
        self.bubbles
    }

    pub fn set_bubbles(&mut self, bubbling: bool) {
        self.bubbles = bubbling;
    }

    pub fn is_cancelable(&self) -> bool {
        self.cancelable
    }

    pub fn set_cancelable(&mut self, cancelable: bool) {
        self.cancelable = cancelable;
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn cancel(&mut self) {
        if self.cancelable {
            self.cancelled = true;
        }
    }

    pub fn is_propagation_stopped(&self) -> bool {
        self.propagation != PropagationMode::AllowPropagation
    }

    pub fn stop_propagation(&mut self) {
        if self.propagation != PropagationMode::StopImmediatePropagation {
            self.propagation = PropagationMode::StopPropagation;
        }
    }

    pub fn is_propagation_stopped_immediately(&self) -> bool {
        self.propagation == PropagationMode::StopImmediatePropagation
    }

    pub fn stop_immediate_propagation(&mut self) {
        self.propagation = PropagationMode::StopImmediatePropagation;
    }

    pub fn phase(&self) -> EventPhase {
        self.event_phase
    }

    pub fn set_phase(&mut self, phase: EventPhase) {
        self.event_phase = phase;
    }

    pub fn target(&self) -> Option<Object<'gc>> {
        self.target
    }

    pub fn set_target(&mut self, target: Object<'gc>) {
        self.target = Some(target)
    }

    pub fn current_target(&self) -> Option<Object<'gc>> {
        self.current_target
    }

    pub fn set_current_target(&mut self, current_target: Object<'gc>) {
        self.current_target = Some(current_target)
    }
}

/// A set of handlers organized by event type, priority, and order added.
#[derive(Clone, Collect, Debug)]
#[collect(no_drop)]
pub struct DispatchList<'gc>(HashMap<AvmString<'gc>, BTreeMap<i32, Vec<EventHandler<'gc>>>>);

impl<'gc> DispatchList<'gc> {
    /// Construct a new dispatch list.
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Get all of the event handlers for a given event type, if such a type
    /// exists.
    fn get_event(
        &self,
        event: impl Into<AvmString<'gc>>,
    ) -> Option<&BTreeMap<i32, Vec<EventHandler<'gc>>>> {
        self.0.get(&event.into())
    }

    /// Get all of the event handlers for a given event type, for mutation.
    ///
    /// If the event type does not exist, it will be added to the dispatch
    /// list.
    fn get_event_mut(
        &mut self,
        event: impl Into<AvmString<'gc>>,
    ) -> &mut BTreeMap<i32, Vec<EventHandler<'gc>>> {
        self.0.entry(event.into()).or_insert_with(BTreeMap::new)
    }

    /// Get a single priority level of event handlers for a given event type,
    /// for mutation.
    fn get_event_priority_mut(
        &mut self,
        event: impl Into<AvmString<'gc>>,
        priority: i32,
    ) -> &mut Vec<EventHandler<'gc>> {
        self.0
            .entry(event.into())
            .or_insert_with(BTreeMap::new)
            .entry(priority)
            .or_insert_with(Vec::new)
    }

    /// Add an event handler to this dispatch list.
    ///
    /// This enforces the invariant that an `EventHandler` must not appear at
    /// more than one priority (since we can't enforce that with clever-er data
    /// structure selection). If an event handler already exists, it will not
    /// be added again, and this function will silently fail.
    pub fn add_event_listener(
        &mut self,
        event: impl Into<AvmString<'gc>> + Clone,
        priority: i32,
        handler: Object<'gc>,
        use_capture: bool,
    ) {
        let new_handler = EventHandler::new(handler, use_capture);

        if let Some(event_sheaf) = self.get_event(event.clone()) {
            for (_other_prio, other_set) in event_sheaf.iter() {
                if other_set.contains(&new_handler) {
                    return;
                }
            }
        }

        self.get_event_priority_mut(event, priority)
            .push(new_handler);
    }

    /// Remove an event handler from this dispatch list.
    ///
    /// Any listener that has the same handler and capture-phase flag will be
    /// removed from any priority in the list.
    pub fn remove_event_listener(
        &mut self,
        event: impl Into<AvmString<'gc>>,
        handler: Object<'gc>,
        use_capture: bool,
    ) {
        let old_handler = EventHandler::new(handler, use_capture);

        for (_prio, set) in self.get_event_mut(event).iter_mut() {
            if let Some(pos) = set.iter().position(|h| *h == old_handler) {
                set.remove(pos);
            }
        }
    }

    /// Determine if there are any event listeners in this dispatch list.
    pub fn has_event_listener(&self, event: impl Into<AvmString<'gc>>) -> bool {
        if let Some(event_sheaf) = self.get_event(event) {
            for (_prio, set) in event_sheaf.iter() {
                if !set.is_empty() {
                    return true;
                }
            }
        }

        false
    }

    /// Yield the event handlers on this dispatch list for a given event.
    ///
    /// Event handlers will be yielded in the order they are intended to be
    /// executed.
    ///
    /// `use_capture` indicates if you want handlers that execute during the
    /// capture phase, or handlers that execute during the bubble and target
    /// phases.
    pub fn iter_event_handlers<'a>(
        &'a mut self,
        event: impl Into<AvmString<'gc>>,
        use_capture: bool,
    ) -> impl 'a + Iterator<Item = Object<'gc>> {
        self.get_event_mut(event)
            .iter()
            .rev()
            .flat_map(|(_p, v)| v.iter())
            .filter(move |eh| eh.use_capture == use_capture)
            .map(|eh| eh.handler)
    }
}

/// A single instance of an event handler.
#[derive(Clone, Collect, Debug)]
#[collect(no_drop)]
struct EventHandler<'gc> {
    /// The event handler to call.
    handler: Object<'gc>,

    /// Indicates if this handler should only be called for capturing events
    /// (when `true`), or if it should only be called for bubbling and
    /// at-target events (when `false`).
    use_capture: bool,
}

impl<'gc> EventHandler<'gc> {
    fn new(handler: Object<'gc>, use_capture: bool) -> Self {
        Self {
            handler,
            use_capture,
        }
    }
}

impl<'gc> PartialEq for EventHandler<'gc> {
    fn eq(&self, rhs: &Self) -> bool {
        self.use_capture == rhs.use_capture && Object::ptr_eq(self.handler, rhs.handler)
    }
}

impl<'gc> Eq for EventHandler<'gc> {}

impl<'gc> Hash for EventHandler<'gc> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.use_capture.hash(state);
        self.handler.as_ptr().hash(state);
    }
}

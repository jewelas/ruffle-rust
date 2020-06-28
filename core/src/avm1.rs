use crate::avm1::globals::create_globals;
use crate::avm1::object::search_prototype;
use crate::context::UpdateContext;
use crate::prelude::*;
use gc_arena::{GcCell, MutationContext};

use swf::avm1::read::Reader;

use crate::display_object::DisplayObject;
use crate::tag_utils::SwfSlice;

#[cfg(test)]
#[macro_use]
mod test_utils;

#[macro_use]
pub mod listeners;

mod activation;
pub mod debug;
pub mod error;
mod fscommand;
pub mod function;
pub mod globals;
pub mod object;
mod property;
mod return_value;
mod scope;
pub mod script_object;
pub mod shared_object;
mod sound_object;
pub mod stack_frame;
mod stage_object;
mod super_object;
mod value;
mod value_object;
pub mod xml_attributes_object;
pub mod xml_idmap_object;
pub mod xml_object;

#[cfg(test)]
mod tests;

use crate::avm1::error::Error;
use crate::avm1::listeners::SystemListener;
use crate::avm1::stack_frame::StackFrame;
pub use activation::Activation;
pub use globals::SystemPrototypes;
pub use object::{Object, ObjectPtr, TObject};
use scope::Scope;
pub use script_object::ScriptObject;
use smallvec::alloc::borrow::Cow;
pub use sound_object::SoundObject;
pub use stage_object::StageObject;
pub use value::Value;

macro_rules! avm_debug {
    ($($arg:tt)*) => (
        #[cfg(feature = "avm_debug")]
        log::debug!($($arg)*)
    )
}

pub struct Avm1<'gc> {
    /// The Flash Player version we're emulating.
    player_version: u8,

    /// The constant pool to use for new activations from code sources that
    /// don't close over the constant pool they were defined with.
    constant_pool: GcCell<'gc, Vec<String>>,

    /// The global object.
    globals: Object<'gc>,

    /// System builtins that we use internally to construct new objects.
    prototypes: globals::SystemPrototypes<'gc>,

    /// System event listeners that will respond to native events (Mouse, Key, etc)
    system_listeners: listeners::SystemListeners<'gc>,

    /// DisplayObject property map.
    display_properties: GcCell<'gc, stage_object::DisplayPropertyMap<'gc>>,

    /// All activation records for the current execution context.
    stack_frames: Vec<GcCell<'gc, Activation<'gc>>>,

    /// The operand stack (shared across functions).
    stack: Vec<Value<'gc>>,

    /// The register slots (also shared across functions).
    /// `ActionDefineFunction2` defined functions do not use these slots.
    registers: [Value<'gc>; 4],

    /// If a serious error has occured, or a user has requested it, the AVM may be halted.
    /// This will completely prevent any further actions from being executed.
    halted: bool,
}

unsafe impl<'gc> gc_arena::Collect for Avm1<'gc> {
    #[inline]
    fn trace(&self, cc: gc_arena::CollectionContext) {
        self.globals.trace(cc);
        self.constant_pool.trace(cc);
        self.system_listeners.trace(cc);
        self.prototypes.trace(cc);
        self.display_properties.trace(cc);
        self.stack_frames.trace(cc);
        self.stack.trace(cc);

        for register in &self.registers {
            register.trace(cc);
        }
    }
}

impl<'gc> Avm1<'gc> {
    pub fn new(gc_context: MutationContext<'gc, '_>, player_version: u8) -> Self {
        let (prototypes, globals, system_listeners) = create_globals(gc_context);

        Self {
            player_version,
            constant_pool: GcCell::allocate(gc_context, vec![]),
            globals,
            prototypes,
            system_listeners,
            display_properties: stage_object::DisplayPropertyMap::new(gc_context),
            stack_frames: vec![],
            stack: vec![],
            registers: [
                Value::Undefined,
                Value::Undefined,
                Value::Undefined,
                Value::Undefined,
            ],
            halted: false,
        }
    }

    #[allow(dead_code)]
    pub fn base_clip(&self) -> DisplayObject<'gc> {
        self.current_stack_frame().unwrap().read().base_clip()
    }

    /// The current target clip for the executing code.
    /// This is the movie clip that contains the bytecode.
    /// Timeline actions like `GotoFrame` use this because
    /// a goto after an invalid tellTarget has no effect.
    pub fn target_clip(&self) -> Option<DisplayObject<'gc>> {
        self.current_stack_frame().unwrap().read().target_clip()
    }

    /// The current target clip of the executing code, or `root` if there is none.
    /// Actions that affect `root` after an invalid `tellTarget` will use this.
    ///
    /// The `root` is determined relative to the base clip that defined the
    pub fn target_clip_or_root(&self) -> DisplayObject<'gc> {
        self.current_stack_frame()
            .unwrap()
            .read()
            .target_clip()
            .unwrap_or_else(|| self.base_clip().root())
    }

    /// Add a stack frame that executes code in timeline scope
    pub fn run_stack_frame_for_action(
        &mut self,
        active_clip: DisplayObject<'gc>,
        swf_version: u8,
        code: SwfSlice,
        action_context: &mut UpdateContext<'_, 'gc, '_>,
    ) {
        if self.halted {
            // We've been told to ignore all future execution.
            return;
        }

        let activation = GcCell::allocate(
            action_context.gc_context,
            Activation::from_nothing(
                swf_version,
                self.global_object_cell(),
                action_context.gc_context,
                active_clip,
            ),
        );
        self.run_with_stack_frame(activation, action_context, |activation, context| {
            let clip_obj = active_clip.object().coerce_to_object(activation, context);
            let child_scope = GcCell::allocate(
                context.gc_context,
                Scope::new(
                    activation.activation().read().scope_cell(),
                    scope::ScopeClass::Target,
                    clip_obj,
                ),
            );
            let child_activation = GcCell::allocate(
                context.gc_context,
                Activation::from_action(
                    swf_version,
                    code,
                    child_scope,
                    activation.avm().constant_pool,
                    active_clip,
                    clip_obj,
                    None,
                ),
            );
            if let Err(e) = activation.avm().run_activation(context, child_activation) {
                root_error_handler(activation, context, e);
            }
        });
    }

    /// Add a stack frame that executes code in timeline scope
    pub fn run_with_stack_frame_for_display_object<'a, F, R>(
        &mut self,
        active_clip: DisplayObject<'gc>,
        swf_version: u8,
        action_context: &mut UpdateContext<'a, 'gc, '_>,
        function: F,
    ) -> R
    where
        for<'b> F: FnOnce(&mut StackFrame<'b, 'gc>, &mut UpdateContext<'a, 'gc, '_>) -> R,
    {
        use crate::tag_utils::SwfMovie;
        use std::sync::Arc;

        let clip_obj = match active_clip.object() {
            Value::Object(o) => o,
            _ => panic!("No script object for display object"),
        };
        let global_scope = GcCell::allocate(
            action_context.gc_context,
            Scope::from_global_object(self.globals),
        );
        let child_scope = GcCell::allocate(
            action_context.gc_context,
            Scope::new(global_scope, scope::ScopeClass::Target, clip_obj),
        );
        let activation = GcCell::allocate(
            action_context.gc_context,
            Activation::from_action(
                swf_version,
                SwfSlice {
                    movie: Arc::new(SwfMovie::empty(swf_version)),
                    start: 0,
                    end: 0,
                },
                child_scope,
                self.constant_pool,
                active_clip,
                clip_obj,
                None,
            ),
        );
        self.run_with_stack_frame(activation, action_context, function)
    }

    /// Add a stack frame that executes code in initializer scope
    pub fn run_stack_frame_for_init_action(
        &mut self,
        active_clip: DisplayObject<'gc>,
        swf_version: u8,
        code: SwfSlice,
        action_context: &mut UpdateContext<'_, 'gc, '_>,
    ) {
        if self.halted {
            // We've been told to ignore all future execution.
            return;
        }

        let activation = GcCell::allocate(
            action_context.gc_context,
            Activation::from_nothing(
                swf_version,
                self.globals,
                action_context.gc_context,
                active_clip,
            ),
        );
        self.run_with_stack_frame(activation, action_context, |activation, context| {
            let clip_obj = active_clip.object().coerce_to_object(activation, context);
            let child_scope = GcCell::allocate(
                context.gc_context,
                Scope::new(
                    activation.activation().read().scope_cell(),
                    scope::ScopeClass::Target,
                    clip_obj,
                ),
            );
            activation.avm().push(Value::Undefined);
            let child_activation = GcCell::allocate(
                context.gc_context,
                Activation::from_action(
                    swf_version,
                    code,
                    child_scope,
                    activation.avm().constant_pool,
                    active_clip,
                    clip_obj,
                    None,
                ),
            );
            if let Err(e) = activation.avm().run_activation(context, child_activation) {
                root_error_handler(activation, context, e);
            }
        });
    }

    /// Add a stack frame that executes code in timeline scope for an object
    /// method, such as an event handler.
    pub fn run_stack_frame_for_method(
        &mut self,
        active_clip: DisplayObject<'gc>,
        obj: Object<'gc>,
        swf_version: u8,
        context: &mut UpdateContext<'_, 'gc, '_>,
        name: &str,
        args: &[Value<'gc>],
    ) {
        if self.halted {
            // We've been told to ignore all future execution.
            return;
        }

        let activation = GcCell::allocate(
            context.gc_context,
            Activation::from_nothing(swf_version, self.globals, context.gc_context, active_clip),
        );
        fn caller<'gc>(
            activation: &mut StackFrame<'_, 'gc>,
            context: &mut UpdateContext<'_, 'gc, '_>,
            obj: Object<'gc>,
            name: &str,
            args: &[Value<'gc>],
        ) {
            let search_result = search_prototype(Some(obj), name, activation, context, obj)
                .and_then(|r| Ok((r.0.resolve(activation, context)?, r.1)));

            if let Ok((callback, base_proto)) = search_result {
                let _ = callback.call(activation, context, obj, base_proto, args);
            }
        }
        self.run_with_stack_frame(activation, context, |activation, context| {
            caller(activation, context, obj, name, args)
        });
    }

    /// Run a function within the scope of an activation.
    pub fn run_with_stack_frame<'a, F, R>(
        &mut self,
        activation: GcCell<'gc, Activation<'gc>>,
        context: &mut UpdateContext<'a, 'gc, '_>,
        function: F,
    ) -> R
    where
        for<'b> F: FnOnce(&mut StackFrame<'b, 'gc>, &mut UpdateContext<'a, 'gc, '_>) -> R,
    {
        self.stack_frames.push(activation);
        let mut stack_frame = StackFrame::new(self, activation);
        // TODO: Handle
        let result = function(&mut stack_frame, context);
        self.stack_frames.pop();
        result
    }

    /// Retrieve the current AVM execution frame.
    ///
    /// Yields None if there is no stack frame.
    pub fn current_stack_frame(&self) -> Result<GcCell<'gc, Activation<'gc>>, Error<'gc>> {
        self.stack_frames.last().copied().ok_or(Error::NoStackFrame)
    }

    /// Checks if there is currently a stack frame.
    ///
    /// This is an indicator if you are currently running from inside or outside the AVM.
    /// This method is cheaper than `current_stack_frame` as it doesn't need to perform a copy.
    pub fn has_stack_frame(&self) -> bool {
        !self.stack_frames.is_empty()
    }

    /// Get the currently executing SWF version.
    pub fn current_swf_version(&self) -> u8 {
        self.current_stack_frame()
            .map(|sf| sf.read().swf_version())
            .unwrap_or(self.player_version)
    }

    /// Returns whether property keys should be case sensitive based on the current SWF version.
    pub fn is_case_sensitive(&self) -> bool {
        is_swf_case_sensitive(self.current_swf_version())
    }

    pub fn notify_system_listeners(
        &mut self,
        active_clip: DisplayObject<'gc>,
        swf_version: u8,
        context: &mut UpdateContext<'_, 'gc, '_>,
        listener: SystemListener,
        method: &str,
        args: &[Value<'gc>],
    ) {
        let activation = GcCell::allocate(
            context.gc_context,
            Activation::from_nothing(swf_version, self.globals, context.gc_context, active_clip),
        );
        self.run_with_stack_frame(activation, context, |activation, context| {
            let listeners = activation.avm().system_listeners.get(listener);
            let mut handlers = listeners.prepare_handlers(activation, context, method);

            for (listener, handler) in handlers.drain(..) {
                let _ = handler.call(activation, context, listener, None, &args);
            }
        });
    }

    /// Execute the AVM stack until a given activation returns.
    pub fn run_activation(
        &mut self,
        context: &mut UpdateContext<'_, 'gc, '_>,
        activation: GcCell<'gc, Activation<'gc>>,
    ) -> Result<(), Error<'gc>> {
        self.stack_frames.push(activation);
        let mut stack_frame = StackFrame::new(self, activation);
        match stack_frame.run(context) {
            Ok(return_type) => {
                self.stack_frames.pop();

                let can_return = activation.read().can_return() && !self.stack_frames.is_empty();
                if can_return {
                    let return_value = return_type.value();
                    activation
                        .write(context.gc_context)
                        .set_return_value(return_value.clone());

                    self.push(return_value);
                }
                Ok(())
            }
            Err(error) => {
                stack_frame.avm().stack_frames.pop();
                if error.is_halting() {
                    stack_frame.avm().halt();
                }
                Err(error)
            }
        }
    }

    /// Halts the AVM, preventing execution of any further actions.
    ///
    /// If the AVM is currently evaluating an action, it will continue until it realizes that it has
    /// been halted. If an immediate stop is required, an Error must be raised inside of the execution.
    ///
    /// This is most often used when serious errors or infinite loops are encountered.
    pub fn halt(&mut self) {
        if !self.halted {
            self.halted = true;
            log::error!("No more actions will be executed in this movie.")
        }
    }

    fn push(&mut self, value: impl Into<Value<'gc>>) {
        let value = value.into();
        avm_debug!("Stack push {}: {:?}", self.stack.len(), value);
        self.stack.push(value);
    }

    #[allow(clippy::let_and_return)]
    fn pop(&mut self) -> Value<'gc> {
        let value = self.stack.pop().unwrap_or_else(|| {
            log::warn!("Avm1::pop: Stack underflow");
            Value::Undefined
        });

        avm_debug!("Stack pop {}: {:?}", self.stack.len(), value);

        value
    }

    /// Obtain the value of `_root`.
    pub fn root_object(&self, _context: &mut UpdateContext<'_, 'gc, '_>) -> Value<'gc> {
        self.base_clip().root().object()
    }

    /// Obtain the value of `_global`.
    pub fn global_object(&self, _context: &mut UpdateContext<'_, 'gc, '_>) -> Value<'gc> {
        Value::Object(self.globals)
    }

    /// Obtain a reference to `_global`.
    pub fn global_object_cell(&self) -> Object<'gc> {
        self.globals
    }

    /// Obtain system built-in prototypes for this instance.
    pub fn prototypes(&self) -> &globals::SystemPrototypes<'gc> {
        &self.prototypes
    }
}

/// Returns whether the given SWF version is case-sensitive.
/// SWFv7 and above is case-sensitive.
pub fn is_swf_case_sensitive(swf_version: u8) -> bool {
    swf_version > 6
}

pub fn root_error_handler<'gc>(
    activation: &mut StackFrame<'_, 'gc>,
    context: &mut UpdateContext<'_, 'gc, '_>,
    error: Error<'gc>,
) {
    if let Error::ThrownValue(error) = error {
        let string = error
            .coerce_to_string(activation, context)
            .unwrap_or_else(|_| Cow::Borrowed("undefined"));
        log::info!(target: "avm_trace", "{}", string);
    } else {
        log::error!("Uncaught error: {:?}", error);
    }
}

/// Utility function used by `Avm1::action_wait_for_frame` and
/// `Avm1::action_wait_for_frame_2`.
fn skip_actions(reader: &mut Reader<'_>, num_actions_to_skip: u8) {
    for _ in 0..num_actions_to_skip {
        if let Err(e) = reader.read_action() {
            log::warn!("Couldn't skip action: {}", e);
        }
    }
}

/// Starts draggining this display object, making it follow the cursor.
/// Runs via the `startDrag` method or `StartDrag` AVM1 action.
pub fn start_drag<'gc>(
    display_object: DisplayObject<'gc>,
    activation: &mut StackFrame<'_, 'gc>,
    context: &mut UpdateContext<'_, 'gc, '_>,
    args: &[Value<'gc>],
) {
    let lock_center = args
        .get(0)
        .map(|o| o.as_bool(context.swf.version()))
        .unwrap_or(false);

    let offset = if lock_center {
        // The object's origin point is locked to the mouse.
        Default::default()
    } else {
        // The object moves relative to current mouse position.
        // Calculate the offset from the mouse to the object in world space.
        let obj_pos = display_object.local_to_global(Default::default());
        (
            obj_pos.0 - context.mouse_position.0,
            obj_pos.1 - context.mouse_position.1,
        )
    };

    let constraint = if args.len() > 1 {
        // Invalid values turn into 0.
        let mut x_min = args
            .get(1)
            .unwrap_or(&Value::Undefined)
            .coerce_to_f64(activation, context)
            .map(|n| if n.is_finite() { n } else { 0.0 })
            .map(Twips::from_pixels)
            .unwrap_or_default();
        let mut y_min = args
            .get(2)
            .unwrap_or(&Value::Undefined)
            .coerce_to_f64(activation, context)
            .map(|n| if n.is_finite() { n } else { 0.0 })
            .map(Twips::from_pixels)
            .unwrap_or_default();
        let mut x_max = args
            .get(3)
            .unwrap_or(&Value::Undefined)
            .coerce_to_f64(activation, context)
            .map(|n| if n.is_finite() { n } else { 0.0 })
            .map(Twips::from_pixels)
            .unwrap_or_default();
        let mut y_max = args
            .get(4)
            .unwrap_or(&Value::Undefined)
            .coerce_to_f64(activation, context)
            .map(|n| if n.is_finite() { n } else { 0.0 })
            .map(Twips::from_pixels)
            .unwrap_or_default();

        // Normalize the bounds.
        if x_max.get() < x_min.get() {
            std::mem::swap(&mut x_min, &mut x_max);
        }
        if y_max.get() < y_min.get() {
            std::mem::swap(&mut y_min, &mut y_max);
        }
        BoundingBox {
            valid: true,
            x_min,
            y_min,
            x_max,
            y_max,
        }
    } else {
        // No constraints.
        Default::default()
    };

    let drag_object = crate::player::DragObject {
        display_object,
        offset,
        constraint,
    };
    *context.drag_object = Some(drag_object);
}

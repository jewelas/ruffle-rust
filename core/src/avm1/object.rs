use crate::avm1::Value;
use crate::display_object::DisplayNode;
use core::fmt;
use gc_arena::{GcCell, MutationContext};
use std::collections::HashMap;

pub type NativeFunction<'gc> =
    fn(MutationContext<'gc, '_>, GcCell<'gc, Object<'gc>>, &[Value<'gc>]) -> Value<'gc>;

pub const TYPE_OF_OBJECT: &str = "object";
pub const TYPE_OF_FUNCTION: &str = "function";
pub const TYPE_OF_MOVIE_CLIP: &str = "movieclip";

#[derive(Clone)]
pub struct Object<'gc> {
    display_node: Option<DisplayNode<'gc>>,
    values: HashMap<String, Value<'gc>>,
    function: Option<NativeFunction<'gc>>,
    type_of: &'static str,
}

unsafe impl<'gc> gc_arena::Collect for Object<'gc> {
    fn trace(&self, cc: gc_arena::CollectionContext) {
        self.display_node.trace(cc);
        self.values.trace(cc);
    }
}

impl fmt::Debug for Object<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Object")
            .field("display_node", &self.display_node)
            .field("values", &self.values)
            .field("function", &self.function.is_some())
            .finish()
    }
}

impl<'gc> Object<'gc> {
    pub fn object() -> Self {
        Self {
            type_of: TYPE_OF_OBJECT,
            display_node: None,
            values: HashMap::new(),
            function: None,
        }
    }

    pub fn function(function: NativeFunction<'gc>) -> Self {
        Self {
            type_of: TYPE_OF_FUNCTION,
            function: Some(function),
            display_node: None,
            values: HashMap::new(),
        }
    }

    pub fn set_display_node(&mut self, display_node: DisplayNode<'gc>) {
        self.display_node = Some(display_node);
    }

    pub fn display_node(&self) -> Option<DisplayNode<'gc>> {
        self.display_node
    }

    pub fn set(&mut self, name: &str, value: Value<'gc>) {
        self.values.insert(name.to_owned(), value);
    }

    pub fn set_function(
        &mut self,
        name: &str,
        function: NativeFunction<'gc>,
        gc_context: MutationContext<'gc, '_>,
    ) {
        self.set(
            name,
            Value::Object(GcCell::allocate(gc_context, Object::function(function))),
        )
    }

    pub fn get(&self, name: &str) -> Value<'gc> {
        if let Some(value) = self.values.get(name) {
            return value.to_owned();
        }
        Value::Undefined
    }

    pub fn has_property(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    pub fn has_own_property(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    pub fn call(
        &self,
        gc_context: MutationContext<'gc, '_>,
        this: GcCell<'gc, Object<'gc>>,
        args: &[Value<'gc>],
    ) -> Value<'gc> {
        if let Some(function) = self.function {
            function(gc_context, this, args)
        } else {
            Value::Undefined
        }
    }

    pub fn set_type_of(&mut self, type_of: &'static str) {
        self.type_of = type_of;
    }

    pub fn type_of(&self) -> &'static str {
        self.type_of
    }
}

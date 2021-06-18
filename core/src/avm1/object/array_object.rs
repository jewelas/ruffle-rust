use crate::avm1::property::Attribute;
use crate::avm1::{Activation, Error, Object, ObjectPtr, ScriptObject, TObject, Value};
use gc_arena::{Collect, GcCell, MutationContext};
use std::borrow::Cow;
use std::fmt;

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
pub struct ArrayObject<'gc>(GcCell<'gc, ScriptObject<'gc>>);

impl fmt::Debug for ArrayObject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ArrayObject").finish()
    }
}

impl<'gc> ArrayObject<'gc> {
    pub fn empty(activation: &Activation<'_, 'gc, '_>) -> Self {
        Self::new(
            activation.context.gc_context,
            activation.context.avm1.prototypes().array,
            [],
        )
    }

    pub fn empty_with_proto(
        gc_context: MutationContext<'gc, '_>,
        proto: Option<Object<'gc>>,
    ) -> Self {
        Self::new_internal(gc_context, proto, [])
    }

    pub fn new(
        gc_context: MutationContext<'gc, '_>,
        array_proto: Object<'gc>,
        elements: impl IntoIterator<Item = Value<'gc>>,
    ) -> Self {
        Self::new_internal(gc_context, Some(array_proto), elements)
    }

    fn new_internal(
        gc_context: MutationContext<'gc, '_>,
        proto: Option<Object<'gc>>,
        elements: impl IntoIterator<Item = Value<'gc>>,
    ) -> Self {
        let base = ScriptObject::object(gc_context, proto);
        let mut length: i32 = 0;
        for value in elements.into_iter() {
            base.define_value(gc_context, &length.to_string(), value, Attribute::empty());
            length += 1;
        }
        base.define_value(
            gc_context,
            "length",
            length.into(),
            Attribute::DONT_ENUM | Attribute::DONT_DELETE,
        );
        Self(GcCell::allocate(gc_context, base))
    }
}

impl<'gc> TObject<'gc> for ArrayObject<'gc> {
    fn get_local(
        &self,
        name: &str,
        activation: &mut Activation<'_, 'gc, '_>,
        this: Object<'gc>,
    ) -> Option<Result<Value<'gc>, Error<'gc>>> {
        self.0.read().get_local(name, activation, this)
    }

    fn set_local(
        &self,
        name: &str,
        value: Value<'gc>,
        activation: &mut Activation<'_, 'gc, '_>,
        this: Object<'gc>,
        base_proto: Option<Object<'gc>>,
    ) -> Result<(), Error<'gc>> {
        if name == "length" {
            let new_length = value.coerce_to_i32(activation)?;
            self.set_length(activation, new_length)?;
        } else if let Ok(index) = name.parse::<i32>() {
            let length = self.length(activation)?;
            if index >= length {
                self.set_length(activation, index.wrapping_add(1))?;
            }
        }

        self.0
            .read()
            .set_local(name, value, activation, this, base_proto)
    }

    fn call(
        &self,
        name: &str,
        activation: &mut Activation<'_, 'gc, '_>,
        this: Object<'gc>,
        base_proto: Option<Object<'gc>>,
        args: &[Value<'gc>],
    ) -> Result<Value<'gc>, Error<'gc>> {
        self.0.read().call(name, activation, this, base_proto, args)
    }

    fn call_setter(
        &self,
        name: &str,
        value: Value<'gc>,
        activation: &mut Activation<'_, 'gc, '_>,
    ) -> Option<Object<'gc>> {
        self.0.read().call_setter(name, value, activation)
    }

    fn create_bare_object(
        &self,
        activation: &mut Activation<'_, 'gc, '_>,
        this: Object<'gc>,
    ) -> Result<Object<'gc>, Error<'gc>> {
        Ok(Self::empty_with_proto(activation.context.gc_context, Some(this)).into())
    }

    fn delete(&self, activation: &mut Activation<'_, 'gc, '_>, name: &str) -> bool {
        self.0.read().delete(activation, name)
    }

    fn add_property(
        &self,
        gc_context: MutationContext<'gc, '_>,
        name: &str,
        get: Object<'gc>,
        set: Option<Object<'gc>>,
        attributes: Attribute,
    ) {
        self.0
            .read()
            .add_property(gc_context, name, get, set, attributes)
    }

    fn add_property_with_case(
        &self,
        activation: &mut Activation<'_, 'gc, '_>,
        name: &str,
        get: Object<'gc>,
        set: Option<Object<'gc>>,
        attributes: Attribute,
    ) {
        self.0
            .read()
            .add_property_with_case(activation, name, get, set, attributes)
    }

    fn set_watcher(
        &self,
        activation: &mut Activation<'_, 'gc, '_>,
        name: Cow<str>,
        callback: Object<'gc>,
        user_data: Value<'gc>,
    ) {
        self.0
            .read()
            .set_watcher(activation, name, callback, user_data);
    }

    fn remove_watcher(&self, activation: &mut Activation<'_, 'gc, '_>, name: Cow<str>) -> bool {
        self.0.read().remove_watcher(activation, name)
    }

    fn define_value(
        &self,
        gc_context: MutationContext<'gc, '_>,
        name: &str,
        value: Value<'gc>,
        attributes: Attribute,
    ) {
        self.0
            .read()
            .define_value(gc_context, name, value, attributes)
    }

    fn set_attributes(
        &self,
        gc_context: MutationContext<'gc, '_>,
        name: Option<&str>,
        set_attributes: Attribute,
        clear_attributes: Attribute,
    ) {
        self.0
            .read()
            .set_attributes(gc_context, name, set_attributes, clear_attributes)
    }

    fn proto(&self) -> Value<'gc> {
        self.0.read().proto()
    }

    fn set_proto(&self, gc_context: MutationContext<'gc, '_>, prototype: Value<'gc>) {
        self.0.read().set_proto(gc_context, prototype);
    }

    fn has_property(&self, activation: &mut Activation<'_, 'gc, '_>, name: &str) -> bool {
        self.0.read().has_property(activation, name)
    }

    fn has_own_property(&self, activation: &mut Activation<'_, 'gc, '_>, name: &str) -> bool {
        self.0.read().has_own_property(activation, name)
    }

    fn has_own_virtual(&self, activation: &mut Activation<'_, 'gc, '_>, name: &str) -> bool {
        self.0.read().has_own_virtual(activation, name)
    }

    fn is_property_enumerable(&self, activation: &mut Activation<'_, 'gc, '_>, name: &str) -> bool {
        self.0.read().is_property_enumerable(activation, name)
    }

    fn get_keys(&self, activation: &mut Activation<'_, 'gc, '_>) -> Vec<String> {
        self.0.read().get_keys(activation)
    }

    fn type_of(&self) -> &'static str {
        self.0.read().type_of()
    }

    fn interfaces(&self) -> Vec<Object<'gc>> {
        self.0.read().interfaces()
    }

    fn set_interfaces(&self, gc_context: MutationContext<'gc, '_>, iface_list: Vec<Object<'gc>>) {
        self.0.read().set_interfaces(gc_context, iface_list)
    }

    fn as_script_object(&self) -> Option<ScriptObject<'gc>> {
        Some(*self.0.read())
    }

    fn as_array_object(&self) -> Option<ArrayObject<'gc>> {
        Some(*self)
    }

    fn as_ptr(&self) -> *const ObjectPtr {
        self.0.read().as_ptr() as *const ObjectPtr
    }

    fn length(&self, activation: &mut Activation<'_, 'gc, '_>) -> Result<i32, Error<'gc>> {
        self.0.read().length(activation)
    }

    fn set_length(
        &self,
        activation: &mut Activation<'_, 'gc, '_>,
        length: i32,
    ) -> Result<(), Error<'gc>> {
        self.0.read().set_length(activation, length)
    }

    fn has_element(&self, activation: &mut Activation<'_, 'gc, '_>, index: i32) -> bool {
        self.0.read().has_element(activation, index)
    }

    fn get_element(&self, activation: &mut Activation<'_, 'gc, '_>, index: i32) -> Value<'gc> {
        self.0.read().get_element(activation, index)
    }

    fn set_element(
        &self,
        activation: &mut Activation<'_, 'gc, '_>,
        index: i32,
        value: Value<'gc>,
    ) -> Result<(), Error<'gc>> {
        let length = self.length(activation)?;
        if index >= length {
            self.set_length(activation, index.wrapping_add(1))?;
        }
        self.0.read().set_element(activation, index, value)
    }

    fn delete_element(&self, activation: &mut Activation<'_, 'gc, '_>, index: i32) -> bool {
        self.0.read().delete_element(activation, index)
    }
}

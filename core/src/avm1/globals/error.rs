//! Error object

use crate::avm1::activation::Activation;
use crate::avm1::error::Error;
use crate::avm1::property::Attribute::*;
use crate::avm1::{Avm1String, Object, ScriptObject, TObject, UpdateContext, Value};
use enumset::EnumSet;
use gc_arena::MutationContext;

pub fn constructor<'gc>(
    activation: &mut Activation<'_, 'gc>,
    context: &mut UpdateContext<'_, 'gc, '_>,
    this: Object<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let message: Value<'gc> = args.get(0).cloned().unwrap_or(Value::Undefined);

    if message != Value::Undefined {
        this.set("message", message, activation, context)?;
    }

    Ok(Value::Undefined)
}

pub fn create_proto<'gc>(
    gc_context: MutationContext<'gc, '_>,
    proto: Object<'gc>,
    fn_proto: Object<'gc>,
) -> Object<'gc> {
    let mut object = ScriptObject::object(gc_context, Some(proto));

    object.define_value(
        gc_context,
        "message",
        Avm1String::new(gc_context, "Error".to_string()).into(),
        EnumSet::empty(),
    );
    object.define_value(
        gc_context,
        "name",
        Avm1String::new(gc_context, "Error".to_string()).into(),
        EnumSet::empty(),
    );

    object.force_set_function(
        "toString",
        to_string,
        gc_context,
        DontDelete | ReadOnly | DontEnum,
        Some(fn_proto),
    );

    object.into()
}

fn to_string<'gc>(
    activation: &mut Activation<'_, 'gc>,
    context: &mut UpdateContext<'_, 'gc, '_>,
    this: Object<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let message = this.get("message", activation, context)?;
    Ok(Avm1String::new(
        context.gc_context,
        message.coerce_to_string(activation, context)?.to_string(),
    )
    .into())
}

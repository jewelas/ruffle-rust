use crate::avm1::{Object, Value, ActionContext, Avm1};
use gc_arena::{MutationContext, GcCell};
use rand::Rng;

mod math;

#[allow(non_snake_case)]
pub fn getURL<'a, 'gc>(
    _avm: &mut Avm1<'gc>,
    context: &mut ActionContext<'a, 'gc, '_>,
    _this: GcCell<'gc, Object<'gc>>,
    args: &[Value<'gc>],
) -> Value<'gc> {
    match args.get(0) {
        Some(url_val) => {
            let url = url_val.clone().into_string();
            let window = args.get(1).map(|v| v.clone().into_string());
            let method = args.get(2).map(|v| v.clone().into_string());

            //TODO: Pull AVM1 locals into key-value storage
            context.navigator.navigate_to_url(url, window, None);
        },
        None => {
            //TODO: Does AVM1 error out?
        }
    }

    Value::Undefined
}

pub fn random<'gc>(
    _avm: &mut Avm1<'gc>,
    action_context: &mut ActionContext<'_, 'gc, '_>,
    _this: GcCell<'gc, Object<'gc>>,
    args: &[Value<'gc>],
) -> Value<'gc> {
    match args.get(0) {
        Some(Value::Number(max)) => Value::Number(action_context.rng.gen_range(0.0f64, max).floor()),
        _ => Value::Undefined //TODO: Shouldn't this be an error condition?
    }
}

pub fn create_globals<'gc>(gc_context: MutationContext<'gc, '_>) -> Object<'gc> {
    let mut globals = Object::object(gc_context);

    globals.set_object("Math", math::create(gc_context));
    globals.set_function("getURL", getURL, gc_context);
    globals.set_function("random", random, gc_context);

    globals
}
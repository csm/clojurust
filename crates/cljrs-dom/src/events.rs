//! DOM event support: event-to-map conversion, `DomListener`, `DomEventChan`.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::ptr::NonNull;
use std::sync::Arc;

use cljrs_async::channel::CljChannel;
use cljrs_env::env::Env;
use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_value::{
    Arity, Keyword, NativeFn, NativeObject, Value, ValueError, ValueResult, gc_native_object,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::DOM_GLOBALS;
use crate::node::dom_node_value;

// ── Active-event pointer (for :prevent-default / :stop-propagation) ──────────

thread_local! {
    /// Raw pointer to the JS Event being dispatched right now. Valid only
    /// during the synchronous body of a DOM event callback.
    pub(crate) static ACTIVE_EVENT: RefCell<Option<NonNull<web_sys::Event>>> =
        const { RefCell::new(None) };
}

fn prevent_default_fn(_args: &[Value]) -> ValueResult<Value> {
    ACTIVE_EVENT.with(|ae| {
        if let Some(ptr) = *ae.borrow() {
            // SAFETY: pointer is valid for the synchronous duration of the callback.
            unsafe { ptr.as_ref() }.prevent_default();
        }
    });
    Ok(Value::Nil)
}

fn stop_propagation_fn(_args: &[Value]) -> ValueResult<Value> {
    ACTIVE_EVENT.with(|ae| {
        if let Some(ptr) = *ae.borrow() {
            unsafe { ptr.as_ref() }.stop_propagation();
        }
    });
    Ok(Value::Nil)
}

fn kw(name: &'static str) -> Value {
    Value::keyword(Keyword::simple(name))
}

// ── Event → Clojure map ───────────────────────────────────────────────────────

/// Convert a `web_sys::Event` to a Clojure map.
///
/// Always includes `:type`, `:target`, `:bubbles`, `:cancelable`,
/// `:prevent-default`, and `:stop-propagation`. Mouse and keyboard events
/// add extra keys.
pub fn event_to_map(event: &web_sys::Event) -> Value {
    let pd = Value::NativeFunction(GcPtr::new(NativeFn::new(
        "prevent-default",
        Arity::Fixed(0),
        prevent_default_fn,
    )));
    let sp = Value::NativeFunction(GcPtr::new(NativeFn::new(
        "stop-propagation",
        Arity::Fixed(0),
        stop_propagation_fn,
    )));

    let target_val = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
        .map(dom_node_value)
        .unwrap_or(Value::Nil);

    let mut map = cljrs_value::MapValue::from_pairs(vec![
        (kw("type"), Value::string(event.type_())),
        (kw("target"), target_val),
        (kw("bubbles"), Value::Bool(event.bubbles())),
        (kw("cancelable"), Value::Bool(event.cancelable())),
        (kw("prevent-default"), pd),
        (kw("stop-propagation"), sp),
    ]);

    if let Some(me) = event.dyn_ref::<web_sys::MouseEvent>() {
        map = map.assoc(kw("client-x"), Value::Long(me.client_x() as i64));
        map = map.assoc(kw("client-y"), Value::Long(me.client_y() as i64));
        map = map.assoc(kw("button"), Value::Long(me.button() as i64));
    } else if let Some(ke) = event.dyn_ref::<web_sys::KeyboardEvent>() {
        map = map.assoc(kw("key"), Value::string(ke.key()));
        map = map.assoc(kw("code"), Value::string(ke.code()));
        map = map.assoc(kw("ctrl-key"), Value::Bool(ke.ctrl_key()));
        map = map.assoc(kw("alt-key"), Value::Bool(ke.alt_key()));
        map = map.assoc(kw("shift-key"), Value::Bool(ke.shift_key()));
        map = map.assoc(kw("meta-key"), Value::Bool(ke.meta_key()));
    }

    Value::Map(map)
}

// ── DomListener ───────────────────────────────────────────────────────────────

pub const DOM_LISTENER_TAG: &str = "DomListener";

/// Holds a live DOM event listener. Removing the event listener and freeing
/// the wasm-bindgen `Closure` both happen in `Drop`.
pub struct DomListener {
    target: web_sys::EventTarget,
    event_type: String,
    callback: js_sys::Function,
    // Kept alive so the Rust closure is not freed while the listener is active.
    _closure: Closure<dyn FnMut(web_sys::Event)>,
}

impl fmt::Debug for DomListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#<DomListener {}>", self.event_type)
    }
}

// SAFETY: wasm32 is single-threaded.
unsafe impl Send for DomListener {}
unsafe impl Sync for DomListener {}

impl Trace for DomListener {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl NativeObject for DomListener {
    fn type_tag(&self) -> &str {
        DOM_LISTENER_TAG
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for DomListener {
    fn drop(&mut self) {
        let _ = self
            .target
            .remove_event_listener_with_callback(&self.event_type, &self.callback);
    }
}

/// Create a managed DOM event listener that calls `handler` (a Clojure fn)
/// with an event map on each event.
///
/// Returns a `Value::NativeObject(DomListener)` that keeps the listener alive
/// and removes it when GC-collected or when `remove_listener` is called.
pub fn create_listener(
    target: &web_sys::EventTarget,
    event_type: String,
    handler: Value,
) -> Result<Value, String> {
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let ptr = NonNull::from(&event);
        ACTIVE_EVENT.with(|ae| *ae.borrow_mut() = Some(ptr));

        let event_map = event_to_map(&event);

        DOM_GLOBALS.with(|g| {
            if let Some(globals) = &*g.borrow() {
                let mut env = Env::new(globals.clone(), "user");
                let _ = cljrs_env::apply::apply_value(&handler, vec![event_map], &mut env);
            }
        });

        ACTIVE_EVENT.with(|ae| *ae.borrow_mut() = None);
    });

    let callback: js_sys::Function = closure.as_ref().unchecked_ref::<js_sys::Function>().clone();

    target
        .add_event_listener_with_callback(&event_type, &callback)
        .map_err(|e| format!("addEventListener failed: {:?}", e))?;

    Ok(Value::NativeObject(gc_native_object(DomListener {
        target: target.clone(),
        event_type,
        callback,
        _closure: closure,
    })))
}

/// Immediately remove the underlying event listener from a `DomListener` value.
///
/// The `DomListener` itself may remain alive until the next GC cycle, but
/// the listener fires no further callbacks after this call.
pub fn remove_listener(val: &Value) -> ValueResult<()> {
    match val {
        Value::NativeObject(ptr) if ptr.get().type_tag() == DOM_LISTENER_TAG => {
            let listener = ptr
                .get()
                .downcast_ref::<DomListener>()
                .expect("DomListener tag but wrong concrete type");
            let _ = listener
                .target
                .remove_event_listener_with_callback(&listener.event_type, &listener.callback);
            Ok(())
        }
        other => Err(ValueError::WrongType {
            expected: "DomListener",
            got: other.type_name().to_string(),
        }),
    }
}

/// Attach an event handler for use inside `dom/render!` (handler is leaked via
/// `Closure::forget` since `render!` does not return a listener handle).
pub fn attach_render_listener(target: &web_sys::EventTarget, event_type: String, handler: Value) {
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let ptr = NonNull::from(&event);
        ACTIVE_EVENT.with(|ae| *ae.borrow_mut() = Some(ptr));

        let event_map = event_to_map(&event);

        DOM_GLOBALS.with(|g| {
            if let Some(globals) = &*g.borrow() {
                let mut env = Env::new(globals.clone(), "user");
                let _ = cljrs_env::apply::apply_value(&handler, vec![event_map], &mut env);
            }
        });

        ACTIVE_EVENT.with(|ae| *ae.borrow_mut() = None);
    });

    let js_fn: &js_sys::Function = closure.as_ref().unchecked_ref();
    let _ = target.add_event_listener_with_callback(&event_type, js_fn);
    // Intentionally leak — the event handler remains active for the page lifetime.
    closure.forget();
}

// ── DomEventChan ─────────────────────────────────────────────────────────────

/// Wraps `Arc<CljChannel>` as a `Value::NativeObject` that the `core.async`
/// builtins (`<!`, `>!`, `close!`) recognise as a regular channel.
///
/// The `as_any` implementation returns `&CljChannel` (not `&DomEventChan`) so
/// that `NativeObjectBox::downcast_ref::<CljChannel>()` succeeds, making this
/// value fully compatible with all channel operations.
pub struct DomEventChan(pub Arc<CljChannel>);

impl fmt::Debug for DomEventChan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#<DomEventChan>")
    }
}

// SAFETY: wasm32 is single-threaded.
unsafe impl Send for DomEventChan {}
unsafe impl Sync for DomEventChan {}

impl Trace for DomEventChan {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl NativeObject for DomEventChan {
    fn type_tag(&self) -> &str {
        // Same tag as CljChannel so core.async builtins accept this value.
        "Channel"
    }

    fn as_any(&self) -> &dyn Any {
        // Return the inner CljChannel so downcast_ref::<CljChannel>() succeeds.
        self.0.as_ref()
    }
}

/// Create a buffered channel that receives event maps whenever `event_type`
/// fires on `target`. The DOM listener is leaked (via `Closure::forget`) so it
/// stays active for the page lifetime.
pub fn create_event_chan(target: &web_sys::EventTarget, event_type: String) -> Value {
    let arc_chan = Arc::new(CljChannel::new(64));
    let closure_chan = arc_chan.clone();

    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let event_map = event_to_map(&event);
        let arc = closure_chan.clone();
        spawn_local(async move {
            arc.put(event_map).await;
        });
    });

    let js_fn: &js_sys::Function = closure.as_ref().unchecked_ref();
    let _ = target.add_event_listener_with_callback(&event_type, js_fn);
    // Leak — the listener (and its Arc<CljChannel> clone) live for the page lifetime.
    closure.forget();

    Value::NativeObject(gc_native_object(DomEventChan(arc_chan)))
}

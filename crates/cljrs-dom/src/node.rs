//! `DomNode` — a `web_sys::Node` wrapped as a Clojure `Value::NativeObject`.

use std::any::Any;
use std::fmt;

use cljrs_gc::{MarkVisitor, Trace};
use cljrs_value::{NativeObject, Value, ValueError, ValueResult, gc_native_object};

pub const DOM_NODE_TAG: &str = "DomNode";

/// Wraps a `web_sys::Node` as a GC-managed Clojure value.
pub struct DomNode(pub web_sys::Node);

impl fmt::Debug for DomNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#<DomNode {}>", self.0.node_name())
    }
}

// SAFETY: wasm32 is single-threaded; these impls are never exercised cross-thread.
unsafe impl Send for DomNode {}
unsafe impl Sync for DomNode {}

impl Trace for DomNode {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl NativeObject for DomNode {
    fn type_tag(&self) -> &str {
        DOM_NODE_TAG
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Wrap a `web_sys::Node` as a `Value::NativeObject`.
pub fn dom_node_value(node: web_sys::Node) -> Value {
    Value::NativeObject(gc_native_object(DomNode(node)))
}

/// Borrow the underlying `web_sys::Node` from a `Value`.
///
/// Returns `Err` if `val` is not a `DomNode` native object.
pub fn as_web_node(val: &Value) -> ValueResult<&web_sys::Node> {
    match val {
        Value::NativeObject(obj) if obj.get().type_tag() == DOM_NODE_TAG => Ok(&obj
            .get()
            .downcast_ref::<DomNode>()
            .expect("DomNode tag but wrong concrete type")
            .0),
        other => Err(ValueError::WrongType {
            expected: "DomNode",
            got: other.type_name().to_string(),
        }),
    }
}

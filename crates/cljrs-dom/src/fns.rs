//! Native functions for the `cljrs.dom` namespace.

use std::sync::Arc;

use cljrs_env::env::GlobalEnv;
use cljrs_gc::GcPtr;
use cljrs_value::{Arity, MapValue, NativeFn, PersistentVector, Value, ValueError, ValueResult};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::events::{attach_render_listener, create_event_chan, create_listener, remove_listener};
use crate::node::{DOM_NODE_TAG, as_web_node, dom_node_value};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn get_document() -> ValueResult<web_sys::Document> {
    web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| ValueError::Other("no browser document available".into()))
}

fn js_err(e: JsValue) -> ValueError {
    ValueError::Other(format!("DOM error: {:?}", e))
}

fn get_str(val: &Value) -> ValueResult<String> {
    match val {
        Value::Str(s) => Ok(s.get().clone()),
        Value::Keyword(k) => Ok(k.get().name.to_string()),
        other => Err(ValueError::WrongType {
            expected: "string",
            got: other.type_name().to_string(),
        }),
    }
}

fn get_node_arg(args: &[Value], idx: usize) -> ValueResult<&web_sys::Node> {
    let val = args
        .get(idx)
        .ok_or_else(|| ValueError::Other("missing argument".into()))?;
    as_web_node(val)
}

fn get_str_arg(args: &[Value], idx: usize) -> ValueResult<String> {
    let val = args
        .get(idx)
        .ok_or_else(|| ValueError::Other("missing argument".into()))?;
    get_str(val)
}

/// Cast a `web_sys::Node` to `web_sys::Element`, erroring if not an element.
fn to_element(node: &web_sys::Node) -> ValueResult<web_sys::Element> {
    node.dyn_ref::<web_sys::Element>()
        .cloned()
        .ok_or_else(|| ValueError::Other("DomNode is not an Element".into()))
}

/// Cast a `web_sys::Node` to `web_sys::HtmlElement`.
fn to_html_element(node: &web_sys::Node) -> ValueResult<web_sys::HtmlElement> {
    node.dyn_ref::<web_sys::HtmlElement>()
        .cloned()
        .ok_or_else(|| ValueError::Other("DomNode is not an HtmlElement".into()))
}

// ── Selection ─────────────────────────────────────────────────────────────────

fn builtin_document(_args: &[Value]) -> ValueResult<Value> {
    let doc = get_document()?;
    let node: web_sys::Node = doc.into();
    Ok(dom_node_value(node))
}

fn builtin_body(_args: &[Value]) -> ValueResult<Value> {
    let doc = get_document()?;
    doc.body()
        .map(|b| dom_node_value(b.unchecked_into::<web_sys::Node>()))
        .ok_or_else(|| ValueError::Other("document has no body".into()))
}

fn builtin_head(_args: &[Value]) -> ValueResult<Value> {
    let doc = get_document()?;
    doc.head()
        .map(|h| dom_node_value(h.unchecked_into::<web_sys::Node>()))
        .ok_or_else(|| ValueError::Other("document has no head".into()))
}

fn builtin_by_id(args: &[Value]) -> ValueResult<Value> {
    let id = get_str_arg(args, 0)?;
    let doc = get_document()?;
    Ok(doc
        .get_element_by_id(&id)
        .map(|el| dom_node_value(el.into()))
        .unwrap_or(Value::Nil))
}

fn builtin_query(args: &[Value]) -> ValueResult<Value> {
    let selector = get_str_arg(args, 0)?;
    let doc = get_document()?;
    Ok(doc
        .query_selector(&selector)
        .map_err(js_err)?
        .map(|el| dom_node_value(el.into()))
        .unwrap_or(Value::Nil))
}

fn builtin_query_all(args: &[Value]) -> ValueResult<Value> {
    let selector = get_str_arg(args, 0)?;
    let doc = get_document()?;
    let list = doc.query_selector_all(&selector).map_err(js_err)?;
    let nodes: Vec<Value> = (0..list.length())
        .filter_map(|i| list.item(i))
        .map(dom_node_value)
        .collect();
    Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(
        nodes,
    ))))
}

// ── Creation ──────────────────────────────────────────────────────────────────

fn builtin_create(args: &[Value]) -> ValueResult<Value> {
    let tag = get_str_arg(args, 0)?;
    let doc = get_document()?;
    let el = doc.create_element(&tag).map_err(js_err)?;
    Ok(dom_node_value(el.into()))
}

fn builtin_create_text(args: &[Value]) -> ValueResult<Value> {
    let text = get_str_arg(args, 0)?;
    let doc = get_document()?;
    let tn = doc.create_text_node(&text);
    Ok(dom_node_value(tn.into()))
}

// ── Tree manipulation ─────────────────────────────────────────────────────────

fn builtin_append(args: &[Value]) -> ValueResult<Value> {
    let parent = get_node_arg(args, 0)?.clone();
    let child = get_node_arg(args, 1)?;
    parent.append_child(child).map_err(js_err)?;
    Ok(args[0].clone())
}

fn builtin_prepend(args: &[Value]) -> ValueResult<Value> {
    let parent = get_node_arg(args, 0)?.clone();
    let child = get_node_arg(args, 1)?;
    let first = parent.first_child();
    parent
        .insert_before(child, first.as_ref())
        .map_err(js_err)?;
    Ok(args[0].clone())
}

fn builtin_remove(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?;
    if let Some(parent) = node.parent_node() {
        parent.remove_child(node).map_err(js_err)?;
    }
    Ok(Value::Nil)
}

fn builtin_replace(args: &[Value]) -> ValueResult<Value> {
    let old = get_node_arg(args, 0)?;
    let new = get_node_arg(args, 1)?;
    if let Some(parent) = old.parent_node() {
        parent.replace_child(new, old).map_err(js_err)?;
    }
    Ok(Value::Nil)
}

fn builtin_parent(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?;
    Ok(node.parent_node().map(dom_node_value).unwrap_or(Value::Nil))
}

fn builtin_children(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?;
    let list = node.child_nodes();
    let nodes: Vec<Value> = (0..list.length())
        .filter_map(|i| list.item(i))
        .map(dom_node_value)
        .collect();
    Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(
        nodes,
    ))))
}

// ── Attributes ────────────────────────────────────────────────────────────────

fn builtin_attr(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?;
    let name = get_str_arg(args, 1)?;
    let el = to_element(node)?;
    Ok(el
        .get_attribute(&name)
        .map(Value::string)
        .unwrap_or(Value::Nil))
}

fn builtin_set_attr(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?.clone();
    let name = get_str_arg(args, 1)?;
    let val = get_str_arg(args, 2)?;
    let el = to_element(&node)?;
    el.set_attribute(&name, &val).map_err(js_err)?;
    Ok(args[0].clone())
}

fn builtin_remove_attr(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?.clone();
    let name = get_str_arg(args, 1)?;
    let el = to_element(&node)?;
    el.remove_attribute(&name).map_err(js_err)?;
    Ok(args[0].clone())
}

// ── Classes ───────────────────────────────────────────────────────────────────

fn builtin_add_class(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?.clone();
    let name = get_str_arg(args, 1)?;
    to_element(&node)?
        .class_list()
        .add_1(&name)
        .map_err(js_err)?;
    Ok(args[0].clone())
}

fn builtin_remove_class(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?.clone();
    let name = get_str_arg(args, 1)?;
    to_element(&node)?
        .class_list()
        .remove_1(&name)
        .map_err(js_err)?;
    Ok(args[0].clone())
}

fn builtin_has_class(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?;
    let name = get_str_arg(args, 1)?;
    Ok(Value::Bool(to_element(node)?.class_list().contains(&name)))
}

fn builtin_toggle_class(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?.clone();
    let name = get_str_arg(args, 1)?;
    to_element(&node)?
        .class_list()
        .toggle(&name)
        .map_err(js_err)?;
    Ok(args[0].clone())
}

// ── Content ───────────────────────────────────────────────────────────────────

fn builtin_text(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?;
    Ok(Value::string(node.text_content().unwrap_or_default()))
}

fn builtin_set_text(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?.clone();
    let text = get_str_arg(args, 1)?;
    node.set_text_content(Some(&text));
    Ok(args[0].clone())
}

fn builtin_html(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?;
    let el = to_element(node)?;
    Ok(Value::string(el.inner_html()))
}

fn builtin_set_html(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?.clone();
    let html = get_str_arg(args, 1)?;
    to_element(&node)?.set_inner_html(&html);
    Ok(args[0].clone())
}

// ── Style & form values ───────────────────────────────────────────────────────

fn builtin_style(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?;
    let prop = get_str_arg(args, 1)?;
    let css = to_html_element(node)?.style();
    Ok(Value::string(
        css.get_property_value(&prop).unwrap_or_default(),
    ))
}

fn builtin_set_style(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?.clone();
    let prop = get_str_arg(args, 1)?;
    let val = get_str_arg(args, 2)?;
    to_html_element(&node)?
        .style()
        .set_property(&prop, &val)
        .map_err(js_err)?;
    Ok(args[0].clone())
}

fn builtin_value(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?;
    let val = if let Some(el) = node.dyn_ref::<web_sys::HtmlInputElement>() {
        el.value()
    } else if let Some(el) = node.dyn_ref::<web_sys::HtmlSelectElement>() {
        el.value()
    } else if let Some(el) = node.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        el.value()
    } else {
        return Err(ValueError::Other(
            "dom/value requires an input, select, or textarea element".into(),
        ));
    };
    Ok(Value::string(val))
}

fn builtin_set_value(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?.clone();
    let val = get_str_arg(args, 1)?;
    if let Some(el) = node.dyn_ref::<web_sys::HtmlInputElement>() {
        el.set_value(&val);
    } else if let Some(el) = node.dyn_ref::<web_sys::HtmlSelectElement>() {
        el.set_value(&val);
    } else if let Some(el) = node.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        el.set_value(&val);
    } else {
        return Err(ValueError::Other(
            "dom/set-value! requires an input, select, or textarea element".into(),
        ));
    }
    Ok(args[0].clone())
}

// ── Events ────────────────────────────────────────────────────────────────────

fn builtin_listen(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?;
    let event_type = get_str_arg(args, 1)?;
    let handler = args
        .get(2)
        .cloned()
        .ok_or_else(|| ValueError::Other("listen! requires a handler fn".into()))?;
    let target: web_sys::EventTarget = node
        .dyn_ref::<web_sys::EventTarget>()
        .cloned()
        .ok_or_else(|| ValueError::Other("DomNode is not an EventTarget".into()))?;
    create_listener(&target, event_type, handler).map_err(ValueError::Other)
}

fn builtin_unlisten(args: &[Value]) -> ValueResult<Value> {
    let val = args
        .first()
        .ok_or_else(|| ValueError::Other("unlisten! requires a DomListener".into()))?;
    remove_listener(val)?;
    Ok(Value::Nil)
}

fn builtin_event_chan(args: &[Value]) -> ValueResult<Value> {
    let node = get_node_arg(args, 0)?;
    let event_type = get_str_arg(args, 1)?;
    let target: web_sys::EventTarget = node
        .dyn_ref::<web_sys::EventTarget>()
        .cloned()
        .ok_or_else(|| ValueError::Other("DomNode is not an EventTarget".into()))?;
    Ok(create_event_chan(&target, event_type))
}

// ── Hiccup renderer ───────────────────────────────────────────────────────────

/// Apply a Clojure attrs map to a DOM element.
fn apply_attrs(el: &web_sys::Element, attrs: &MapValue) -> ValueResult<()> {
    for (k, v) in attrs.iter() {
        let attr_name: String = match k {
            Value::Keyword(kw) => kw.get().name.to_string(),
            Value::Str(s) => s.get().clone(),
            _ => continue,
        };

        match attr_name.as_str() {
            "style" => {
                if let Value::Map(style_map) = v {
                    let html_el: web_sys::HtmlElement = el
                        .clone()
                        .dyn_into::<web_sys::HtmlElement>()
                        .map_err(|_| ValueError::Other("element is not an HtmlElement".into()))?;
                    let css = html_el.style();
                    for (sk, sv) in style_map.iter() {
                        let prop: String = match sk {
                            Value::Keyword(kw) => kw.get().name.to_string(),
                            Value::Str(s) => s.get().clone(),
                            _ => continue,
                        };
                        let val_str: String = match sv {
                            Value::Str(s) => s.get().clone(),
                            other => format!("{other}"),
                        };
                        css.set_property(&prop, &val_str).map_err(js_err)?;
                    }
                }
            }
            name if name.starts_with("on-") => {
                let event_type = name[3..].to_string();
                let target: web_sys::EventTarget = el
                    .clone()
                    .dyn_into::<web_sys::EventTarget>()
                    .map_err(|_| ValueError::Other("element is not an EventTarget".into()))?;
                attach_render_listener(&target, event_type, v.clone());
            }
            _ => {
                let val_str = match v {
                    Value::Nil => continue,
                    Value::Str(s) => s.get().clone(),
                    other => format!("{other}"),
                };
                el.set_attribute(&attr_name, &val_str).map_err(js_err)?;
            }
        }
    }
    Ok(())
}

/// Recursively convert a hiccup `Value` to a `web_sys::Node`.
fn render_node(doc: &web_sys::Document, val: &Value) -> ValueResult<web_sys::Node> {
    match val {
        Value::Vector(v) => render_hiccup(doc, v.get()),
        Value::Str(s) => Ok(doc.create_text_node(s.get()).into()),
        Value::NativeObject(obj) if obj.get().type_tag() == DOM_NODE_TAG => {
            Ok(as_web_node(val)?.clone())
        }
        other => {
            let text = format!("{other}");
            Ok(doc.create_text_node(&text).into())
        }
    }
}

/// Render a hiccup vector `[:tag {attrs} & children]` into a `web_sys::Node`.
fn render_hiccup(doc: &web_sys::Document, v: &PersistentVector) -> ValueResult<web_sys::Node> {
    if v.count() == 0 {
        return Err(ValueError::Other("hiccup vector cannot be empty".into()));
    }

    let tag = match v.nth(0) {
        Some(Value::Keyword(k)) => k.get().name.to_string(),
        Some(Value::Str(s)) => s.get().clone(),
        _ => {
            return Err(ValueError::Other(
                "hiccup tag must be a keyword or string".into(),
            ));
        }
    };

    let el = doc.create_element(&tag).map_err(js_err)?;

    let children_start = if v.count() > 1 {
        match v.nth(1) {
            Some(Value::Map(attrs)) => {
                apply_attrs(&el, attrs)?;
                2
            }
            Some(Value::Nil) => 2,
            _ => 1,
        }
    } else {
        1
    };

    for i in children_start..v.count() {
        if let Some(child_val) = v.nth(i) {
            let child_node = render_node(doc, child_val)?;
            el.append_child(&child_node).map_err(js_err)?;
        }
    }

    Ok(el.into())
}

fn builtin_render(args: &[Value]) -> ValueResult<Value> {
    let parent_node = get_node_arg(args, 0)?.clone();
    let hiccup = args
        .get(1)
        .ok_or_else(|| ValueError::Other("render! requires a hiccup vector".into()))?;

    // Clear existing children of the parent element.
    while let Some(child) = parent_node.first_child() {
        parent_node.remove_child(&child).map_err(js_err)?;
    }

    let doc = get_document()?;
    let new_child = render_node(&doc, hiccup)?;
    parent_node.append_child(&new_child).map_err(js_err)?;

    Ok(args[0].clone())
}

// ── Registration ──────────────────────────────────────────────────────────────

pub fn register(globals: &Arc<GlobalEnv>) {
    let ns = "cljrs.dom";
    globals.get_or_create_ns(ns);

    let fns: Vec<(&str, Arity, fn(&[Value]) -> ValueResult<Value>)> = vec![
        // Selection
        ("document", Arity::Fixed(0), builtin_document),
        ("body", Arity::Fixed(0), builtin_body),
        ("head", Arity::Fixed(0), builtin_head),
        ("by-id", Arity::Fixed(1), builtin_by_id),
        ("query", Arity::Fixed(1), builtin_query),
        ("query-all", Arity::Fixed(1), builtin_query_all),
        // Creation
        ("create", Arity::Fixed(1), builtin_create),
        ("create-text", Arity::Fixed(1), builtin_create_text),
        // Tree manipulation
        ("append!", Arity::Fixed(2), builtin_append),
        ("prepend!", Arity::Fixed(2), builtin_prepend),
        ("remove!", Arity::Fixed(1), builtin_remove),
        ("replace!", Arity::Fixed(2), builtin_replace),
        ("parent", Arity::Fixed(1), builtin_parent),
        ("children", Arity::Fixed(1), builtin_children),
        // Attributes
        ("attr", Arity::Fixed(2), builtin_attr),
        ("set-attr!", Arity::Fixed(3), builtin_set_attr),
        ("remove-attr!", Arity::Fixed(2), builtin_remove_attr),
        // Classes
        ("add-class!", Arity::Fixed(2), builtin_add_class),
        ("remove-class!", Arity::Fixed(2), builtin_remove_class),
        ("has-class?", Arity::Fixed(2), builtin_has_class),
        ("toggle-class!", Arity::Fixed(2), builtin_toggle_class),
        // Content
        ("text", Arity::Fixed(1), builtin_text),
        ("set-text!", Arity::Fixed(2), builtin_set_text),
        ("html", Arity::Fixed(1), builtin_html),
        ("set-html!", Arity::Fixed(2), builtin_set_html),
        // Style & form
        ("style", Arity::Fixed(2), builtin_style),
        ("set-style!", Arity::Fixed(3), builtin_set_style),
        ("value", Arity::Fixed(1), builtin_value),
        ("set-value!", Arity::Fixed(2), builtin_set_value),
        // Events
        ("listen!", Arity::Fixed(3), builtin_listen),
        ("unlisten!", Arity::Fixed(1), builtin_unlisten),
        ("event-chan", Arity::Fixed(2), builtin_event_chan),
        // Hiccup
        ("render!", Arity::Fixed(2), builtin_render),
    ];

    for (name, arity, func) in fns {
        let nf = NativeFn::new(name, arity, func);
        globals.intern(ns, Arc::from(name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

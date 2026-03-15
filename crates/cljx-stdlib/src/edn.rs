//! Native implementations for `clojure.edn`.
//!
//! EDN reading produces data values only — no evaluation, no reader macros
//! like `#()` or `@`.  We use the existing parser + `form_to_value` which
//! is slightly looser than strict EDN but covers all practical cases.

use std::sync::Arc;

use cljx_eval::eval::form_to_value;
use cljx_reader::Parser;
use cljx_value::{Arity, Keyword, ResourceHandle, Value, ValueError, ValueResult};

use crate::io::{IoReader, StringReader};
use crate::register_fns;

pub fn register(globals: &Arc<cljx_eval::GlobalEnv>, ns: &str) {
    register_fns!(
        globals,
        ns,
        [
            ("read-string", Arity::Variadic { min: 1 }, edn_read_string),
            ("read", Arity::Variadic { min: 1 }, edn_read),
        ]
    );
}

/// Parse the `:eof` value from an opts map, if present.
fn get_eof_value(opts: &Value) -> Option<Value> {
    if let Value::Map(m) = opts {
        let eof_key = Value::keyword(Keyword::simple("eof"));
        let mut result = None;
        m.for_each(|k, v| {
            if *k == eof_key {
                result = Some(v.clone());
            }
        });
        result
    } else {
        None
    }
}

/// Read one EDN form from a string.
///
/// `(clojure.edn/read-string s)`
/// `(clojure.edn/read-string opts s)`
fn edn_read_string(args: &[Value]) -> ValueResult<Value> {
    let (opts, s) = match args.len() {
        1 => (None, &args[0]),
        2 => (Some(&args[0]), &args[1]),
        n => {
            return Err(ValueError::ArityError {
                name: "clojure.edn/read-string".into(),
                expected: "1-2".into(),
                got: n,
            })
        }
    };

    let src = match s {
        Value::Str(s) => s.get().clone(),
        Value::Nil => return Ok(Value::Nil),
        v => {
            return Err(ValueError::WrongType {
                expected: "string",
                got: v.type_name().to_string(),
            })
        }
    };

    let eof_val = opts.and_then(get_eof_value);

    let mut parser = Parser::new(src, "<edn>".to_string());
    match parser.parse_one() {
        Ok(Some(form)) => Ok(form_to_value(&form)),
        Ok(None) => match eof_val {
            Some(v) => Ok(v),
            None => Err(ValueError::Other("EOF while reading EDN".into())),
        },
        Err(e) => Err(ValueError::Other(format!("EDN parse error: {e}"))),
    }
}

/// Read one EDN form from a reader resource.
///
/// `(clojure.edn/read reader)`
/// `(clojure.edn/read opts reader)`
fn edn_read(args: &[Value]) -> ValueResult<Value> {
    let (opts, reader_val) = match args.len() {
        1 => (None, &args[0]),
        2 => (Some(&args[0]), &args[1]),
        n => {
            return Err(ValueError::ArityError {
                name: "clojure.edn/read".into(),
                expected: "1-2".into(),
                got: n,
            })
        }
    };

    let eof_val = opts.and_then(get_eof_value);

    let src = match reader_val {
        Value::Resource(r) => read_all_from_resource(r)?,
        v => {
            return Err(ValueError::WrongType {
                expected: "reader",
                got: v.type_name().to_string(),
            })
        }
    };

    let mut parser = Parser::new(src, "<edn>".to_string());
    match parser.parse_one() {
        Ok(Some(form)) => Ok(form_to_value(&form)),
        Ok(None) => match eof_val {
            Some(v) => Ok(v),
            None => Err(ValueError::Other("EOF while reading EDN".into())),
        },
        Err(e) => Err(ValueError::Other(format!("EDN parse error: {e}"))),
    }
}

fn read_all_from_resource(r: &ResourceHandle) -> ValueResult<String> {
    if let Some(reader) = r.downcast::<IoReader>() {
        reader.read_all()
    } else if let Some(reader) = r.downcast::<StringReader>() {
        reader.read_all()
    } else {
        Err(ValueError::Other("not a readable resource".into()))
    }
}

use cljrs_gc::GcPtr;
use cljrs_value::regex::{MatchPhase, Matcher};
use cljrs_value::{Value, ValueError, ValueResult};

pub fn builtin_re_pattern(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Pattern(pattern) => Ok(Value::Pattern(pattern.clone())),
        Value::Matcher(matcher) => Ok(Value::Pattern(matcher.get().pattern.clone())),
        Value::Str(s) => {
            let pattern = regex::Regex::new(s.get().as_str());
            match pattern {
                Ok(pattern) => Ok(Value::Pattern(GcPtr::new(pattern))),
                Err(e) => Err(ValueError::Other(e.to_string())),
            }
        }
        v => Err(ValueError::WrongType {
            expected: "str",
            got: v.type_name().to_string(),
        }),
    }
}

fn get_match(matcher: &Matcher) -> ValueResult<Value> {
    let state = matcher.next();
    match state {
        MatchPhase::Matching(_) => match matcher.capture() {
            Some(c) => Ok(c.to_value()),
            None => Ok(Value::Nil),
        },
        _ => Ok(Value::Nil),
    }
}

pub fn builtin_re_find(args: &[Value]) -> ValueResult<Value> {
    if args.len() == 1 {
        match &args[0] {
            Value::Matcher(m) => get_match(m.get()),
            v => Err(ValueError::WrongType {
                expected: "matcher",
                got: v.type_name().to_string(),
            }),
        }
    } else if args.len() == 2 {
        let matcher = new_matcher(args, false)?;
        get_match(&matcher)
    } else {
        Err(ValueError::ArityError {
            name: "re-find".to_string(),
            expected: "1 or 2".to_string(),
            got: args.len(),
        })
    }
}

pub fn builtin_re_matches(args: &[Value]) -> ValueResult<Value> {
    let matcher = new_matcher(args, true)?;
    matcher.next();
    Ok(match matcher.capture() {
        Some(c) => c.to_value(),
        None => Value::Nil,
    })
}

pub fn builtin_re_groups(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Matcher(m) => match m.get().capture() {
            Some(c) => Ok(c.to_value()),
            None => Ok(Value::Nil),
        },
        v => Err(ValueError::WrongType {
            expected: "matcher",
            got: v.type_name().to_string(),
        }),
    }
}

fn new_matcher(args: &[Value], match_all: bool) -> ValueResult<Matcher> {
    let pattern = match &args[0] {
        Value::Pattern(p) => Ok(p.get().clone()),
        Value::Str(s) => regex::Regex::new(s.get()),
        v => {
            return Err(ValueError::WrongType {
                expected: "str or pattern",
                got: v.type_name().to_string(),
            });
        }
    };
    let haystack = match &args[1] {
        Value::Str(s) => s.get().to_string(),
        v => {
            return Err(ValueError::WrongType {
                expected: "str",
                got: v.type_name().to_string(),
            });
        }
    };
    match pattern {
        Ok(pattern) => Ok(Matcher::new(pattern, haystack, match_all)),
        Err(e) => Err(ValueError::Other(e.to_string())),
    }
}

pub fn builtin_re_matcher(args: &[Value]) -> ValueResult<Value> {
    new_matcher(args, false).map(|m| Value::Matcher(GcPtr::new(m)))
}

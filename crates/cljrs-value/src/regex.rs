use std::sync::Mutex;
use regex::{Captures, Regex};
use cljrs_gc::{GcPtr, GcVisitor, MarkVisitor, Trace};
use crate::{PersistentVector, Value};

#[derive(Debug, Clone)]
pub enum MatchPhase {
    New,
    Matching(usize),
    Complete,
}

#[derive(Debug, Clone)]
struct MatcherState {
    phase: MatchPhase,
    last_match: Option<MatchResult>,
}

#[derive(Debug)]
pub struct Matcher {
    pub pattern: GcPtr<Regex>,
    haystack: GcPtr<String>,
    state: Mutex<MatcherState>,
    match_all: bool,
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub full: String,
    pub groups: Vec<Option<String>>,
}

impl Clone for Matcher {
    fn clone(&self) -> Matcher {
        let state = self.state.lock().unwrap().clone();
        Matcher {
            pattern: self.pattern.clone(),
            haystack: self.haystack.clone(),
            state: Mutex::new(state.clone()),
            match_all: self.match_all,
        }
    }
}

impl Trace for Matcher {
    fn trace(&self, visitor: &mut MarkVisitor) {
        visitor.visit(&self.pattern);
        visitor.visit(&self.haystack);
    }
}

impl Matcher {
    pub fn new(pattern: Regex, source: String, match_all: bool) -> Self {
        Self {
            pattern: GcPtr::new(pattern),
            haystack: GcPtr::new(source),
            state: Mutex::new(MatcherState {
                phase: MatchPhase::New,
                last_match: None,
            }),
            match_all,
        }
    }

    pub fn next(&self) -> MatchPhase {
        let mut state = self.state.lock().unwrap();
        match state.phase {
            MatchPhase::New => {
                match self.pattern.get().captures(self.haystack.get()) {
                    Some(cap) => if !self.match_all || cap.len() == self.haystack.get().len() {
                        let match_ = cap.get_match();
                        *state = MatcherState {
                            phase: MatchPhase::Matching(match_.end()),
                            last_match: Some(MatchResult::new(cap))
                        }
                    }
                    None => {
                        *state = MatcherState {
                            phase: MatchPhase::Complete,
                            last_match: None,
                        }
                    }
                }
            }
            MatchPhase::Matching(n) => {
                match self.pattern.get().captures_at(self.haystack.get(), n) {
                    Some(cap) => {
                        *state = MatcherState {
                            phase: MatchPhase::Matching(cap.get_match().end()),
                            last_match: Some(MatchResult::new(cap))
                        }
                    }
                    None => {
                        *state = MatcherState {
                            phase: MatchPhase::Complete,
                            last_match: None,
                        };
                    }
                }
            }
            MatchPhase::Complete => {}
        }
        state.phase.clone()
    }

    pub fn capture(&self) -> Option<MatchResult> {
        let state = self.state.lock().unwrap();
        state.last_match.clone()
    }

    pub fn phase(&self) -> MatchPhase {
        self.state.lock().unwrap().phase.clone()
    }
}

impl MatchResult {
    pub fn new(cap: Captures) -> Self {
        Self {
            full: cap.get_match().as_str().to_string(),
            groups: cap.iter().map(|g| g.map(|e| e.as_str().to_string())).collect(),
        }
    }

    pub fn to_value(&self) -> Value {
        if self.groups.len() == 1 || self.groups.iter().skip(1).all(|g| g.is_none()) {
            Value::Str(GcPtr::new(self.full.to_string()))
        } else {
            let groups: Vec<Value> = self.groups.iter().map(|g| {
                match g {
                    Some(m) => Value::Str(GcPtr::new(m.to_string())),
                    None => Value::Nil
                }
            }).collect();
            Value::Vector(GcPtr::new(PersistentVector::from_iter(groups)))
        }
    }
}

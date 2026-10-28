//! Adapters used by typed exports at the Clojure `blocking` boundary.
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::{Value, ValueError, ValueResult};

/// Generated code borrows this candidate to select a Send-capable adapter.
/// The conversion callback remains on the isolate and needs no Send bound.
#[doc(hidden)]
pub struct BlockingCall<A, F, C> {
    args: RefCell<Option<A>>,
    name: &'static str,
    job: RefCell<Option<F>>,
    convert: RefCell<Option<C>>,
}
impl<A, F, C, T> BlockingCall<A, F, C>
where
    F: FnOnce(A) -> T,
    C: FnOnce(T) -> ValueResult<Value>,
{
    pub fn new(name: &'static str, args: A, job: F, convert: C) -> Self {
        Self {
            args: RefCell::new(Some(args)),
            name,
            job: RefCell::new(Some(job)),
            convert: RefCell::new(Some(convert)),
        }
    }
}

/// Autoref dispatch keeps non-Send exports usable outside `blocking`.
/// Generated monomorphic code selects the worker path only for owned Send jobs.
#[doc(hidden)]
pub trait BlockingDispatch {
    fn run_blocking(self) -> ValueResult<Value>;
}
impl<A, F, C, T> BlockingDispatch for &BlockingCall<A, F, C>
where
    A: Send + 'static,
    F: FnOnce(A) -> T + Send + 'static,
    T: Send + 'static,
    C: FnOnce(T) -> ValueResult<Value>,
{
    fn run_blocking(self) -> ValueResult<Value> {
        let job = self
            .job
            .borrow_mut()
            .take()
            .expect("blocking call already consumed");
        let args = self.args.borrow_mut().take().unwrap();
        let outcome =
            cljrs_value::parking::blocking(move || catch_unwind(AssertUnwindSafe(|| job(args))));
        match outcome {
            Ok(output) => {
                let convert = self.convert.borrow_mut().take().unwrap();
                convert(output)
            }
            Err(payload) => {
                let message = payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("non-string panic");
                Err(ValueError::Other(format!(
                    "blocking native {} panicked: {message}",
                    self.name
                )))
            }
        }
    }
}
// Method lookup adds one reference only when the Send implementation is unavailable.
impl<A, F, C> BlockingDispatch for &&BlockingCall<A, F, C> {
    fn run_blocking(self) -> ValueResult<Value> {
        Err(ValueError::Other(format!(
            "blocking cannot offload {}: inputs and output must be owned, Send, and 'static",
            self.name
        )))
    }
}

use cljrs_async::{await_value, spawn_future};
use cljrs_runtime::env::error::EvalError;

fn block_on_local<F: std::future::Future>(future: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    tokio::task::LocalSet::new().block_on(&runtime, future)
}

#[test]
fn spawned_work_charges_every_captured_meter() {
    block_on_local(async {
        let outer = cljrs_runtime::env::gas::GasMeter::new(3);
        let inner = cljrs_runtime::env::gas::GasMeter::new(2);
        let future = {
            let _outer = cljrs_runtime::env::gas::GasGuard::install(outer.clone());
            let _inner = cljrs_runtime::env::gas::GasGuard::install(inner.clone());
            spawn_future(async {
                tokio::task::yield_now().await;
                if cljrs_runtime::env::gas::charge(2) {
                    Ok(cljrs_value::Value::Nil)
                } else {
                    Err(EvalError::GasExhausted)
                }
            })
        };

        await_value(future).await.expect("within both budgets");
        assert_eq!(outer.remaining(), 1);
        assert_eq!(inner.remaining(), 0);
    });
}

/// A task that yields once and then reaches a checkpoint.
fn checkpoint_after_yield() -> cljrs_value::Value {
    spawn_future(async {
        tokio::task::yield_now().await;
        if cljrs_runtime::env::gas::charge(1) {
            Ok(cljrs_value::Value::Nil)
        } else {
            Err(EvalError::GasExhausted)
        }
    })
}

/// An earlier evaluation's task is polled while a later, interrupted
/// evaluation drives the `LocalSet`. Only the interrupted evaluation's own
/// task stops.
#[test]
fn an_interrupt_stops_only_tasks_of_the_interrupted_evaluation() {
    use cljrs_runtime::env::gas::InterruptFlag;
    use std::sync::Arc;

    block_on_local(async {
        let bystander = checkpoint_after_yield();

        let flag = Arc::new(InterruptFlag::new(true));
        let _guard = cljrs_runtime::env::gas::InterruptGuard::install(flag);
        let own = checkpoint_after_yield();

        await_value(bystander)
            .await
            .expect("nobody interrupted the evaluation that spawned this task");
        assert!(matches!(
            await_value(own).await,
            Err(EvalError::GasExhausted)
        ));
        assert!(cljrs_runtime::env::gas::interrupt_requested());
    });
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

    /// Several evaluations each spawn a task, then one more drives them all.
    /// A task stops exactly when the evaluation that spawned it was
    /// interrupted, whatever the driver's own flag says.
    #[test]
    fn a_task_stops_iff_its_own_evaluation_was_interrupted(
        spawners in proptest::collection::vec(proptest::bool::ANY, 1..6),
        driver in proptest::bool::ANY,
    ) {
        use std::sync::Arc;
        use cljrs_runtime::env::gas::InterruptFlag;
        use cljrs_runtime::env::gas::InterruptGuard;

        let stopped = block_on_local(async {
            let tasks: Vec<cljrs_value::Value> = spawners
                .iter()
                .map(|interrupted| {
                    let _eval = InterruptGuard::install(Arc::new(InterruptFlag::new(*interrupted)));
                    checkpoint_after_yield()
                })
                .collect();
            // A settled future is no longer rooted by its task.
            let _tasks_root = cljrs_runtime::env::gc_roots::root_values(&tasks);
            let _driver = InterruptGuard::install(Arc::new(InterruptFlag::new(driver)));
            let mut stopped = Vec::with_capacity(tasks.len());
            for task in &tasks {
                let outcome = await_value(task.clone()).await;
                stopped.push(matches!(outcome, Err(EvalError::GasExhausted)));
            }
            stopped
        });
        proptest::prop_assert_eq!(stopped, spawners);
    }
}

#[test]
fn future_gas_exhaustion_stays_non_catchable_error() {
    block_on_local(async {
        let meter = cljrs_runtime::env::gas::GasMeter::new(0);
        let future = {
            let _guard = cljrs_runtime::env::gas::GasGuard::install(meter);
            spawn_future(async {
                if cljrs_runtime::env::gas::charge(1) {
                    Ok(cljrs_value::Value::Nil)
                } else {
                    Err(EvalError::GasExhausted)
                }
            })
        };

        assert!(matches!(
            await_value(future).await,
            Err(EvalError::GasExhausted)
        ));
    });
}

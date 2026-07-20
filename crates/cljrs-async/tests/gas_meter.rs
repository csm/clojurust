use cljrs_async::{await_value, spawn_future};
use cljrs_env::error::EvalError;

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
        let outer = cljrs_env::gas::GasMeter::new(3);
        let inner = cljrs_env::gas::GasMeter::new(2);
        let future = {
            let _outer = cljrs_env::gas::GasGuard::install(outer.clone());
            let _inner = cljrs_env::gas::GasGuard::install(inner.clone());
            spawn_future(async {
                tokio::task::yield_now().await;
                if cljrs_env::gas::charge(2) {
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

#[test]
fn future_gas_exhaustion_stays_non_catchable_error() {
    block_on_local(async {
        let meter = cljrs_env::gas::GasMeter::new(0);
        let future = {
            let _guard = cljrs_env::gas::GasGuard::install(meter);
            spawn_future(async {
                if cljrs_env::gas::charge(1) {
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

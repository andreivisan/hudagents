# FSM + DAG Planner

## Goal

Build a planner-only library that provides:
- topological ordering for DAG nodes (`kahn`)
- conditional node enablement (`Cond<A>` + `AtomEval<S, C>`)
- pass-based planning without execution side effects (`run_passes`)

This crate decides **what can run next**, not **how it runs**.

## Client -> stack trace

### Client code (what the framework user writes)

```rust
use fsm_dag::{AtomEval, Cond, PassView, run_passes};

#[derive(Clone, Copy)]
enum Phase { Plan, Run, Done }

#[derive(Default)]
struct WorkflowCtx { counter: u32 }

enum FlowAtom {
    IsPlanning,
    CounterLt(u32),
}

impl AtomEval<Phase, WorkflowCtx> for FlowAtom {
    fn eval(&self, state: &Phase, ctx: &WorkflowCtx) -> bool {
        match self {
            FlowAtom::IsPlanning => matches!(state, Phase::Plan),
            FlowAtom::CounterLt(n) => ctx.counter < *n,
        }
    }
}

let enabled = vec![
    Cond::Atom(FlowAtom::IsPlanning),
    Cond::Always,
    Cond::Atom(FlowAtom::CounterLt(2)),
];
let edges = vec![(0, 2), (1, 2)];
let states = [Phase::Plan, Phase::Run];
let ctxs = [WorkflowCtx { counter: 0 }, WorkflowCtx { counter: 1 }];
let pass_views = vec![
    PassView { state: &states[0], ctx: &ctxs[0] },
    PassView { state: &states[1], ctx: &ctxs[1] },
];

let plan = run_passes(3, &edges, &enabled, &pass_views, 8);
println!("{:?}", plan.stop_reason);
```

### Stack trace (what happens internally)

```run_passes -> validate input -> kahn (topological order) -> for each PassView: run -> Cond::eval -> collect enabled node ids -> stop at NoProgress / HitMaxPasses / ExhaustedPassViews / CycleDetected```

## API quick map

- `Cond<A>`: composable condition tree (`Always`, `Atom`, `And`, `Or`, `Not`)
- `AtomEval<S, C>`: condition atom contract for `(state, context)`
- `kahn(num_nodes, edges)`: returns topological order or `[]` when cyclic/invalid
- `run(state, ctx, topo_order, enabled)`: returns enabled node ids in topo order
- `run_passes(...) -> PassPlan`: multi-pass planning with explicit `StopReason`

## Design note

`fsm_dag` is intentionally decoupled from execution/runtime concerns.
Execution (actors, timeouts, retries, backpressure) belongs to `workflow-orchestrator` + `actor_model`.

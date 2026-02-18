# Workflow Engine

## Goal

Build a Workflow Runner that composes:
    - your fsm-dag library (topological ordering + conditional enablement)
    - your actor-model library (agents/tools/group-chat executed via handles)
    - a tiny FSM (Phase::Plan → Phase::Run → Phase::Done) to allow repeated DAG passes (cycles) safely

## Client → stack trace

### Client code (what the framework user writes)

```rust
// 1) Spawn actors (from actor-model crate)
let alice = spawn_echo_agent("alice", 8, RestartPolicy::Never).await?;
let bob   = spawn_echo_agent("bob", 8, RestartPolicy::Never).await?;
let team  = spawn_group_manager(vec![alice.clone(), bob.clone()], 8, RestartPolicy::Never).await?;

// 2) Build a registry (maps ids -> handles)
let mut reg = Registry::default();
reg.insert_agent("alice", alice);
reg.insert_agent("bob", bob);
reg.insert_manager("team", team);

// 3) Build workflow spec (nodes/edges/conditions)
let wf: WorkflowSpec<FlowAtom> = build_demo_workflow();

// 4) Run workflow
let mut ctx = WorkflowCtx::new();
let final_out = run_workflow(&wf, &reg, "hello".to_string(), &mut ctx, RunLimits::default()).await?;
println!("{final_out}");
```

### Stack trace (what happens internally)

```run_workflow → plan_pass (fsm_dag::kahn + fsm_dag::run) → execute_runnable_nodes → per node: resolve_input → call actor handle → write_output → update phase → next pass until Done/stop```

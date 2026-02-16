# Actor Runtime

## Goal

Build a local actor runtime that provides:
- bounded mailbox actors (`spawn_actor`)
- request/reply over channels with timeout support
- send policies (`Backpressure`, `FailFast`)
- supervisor-based restart policy on panic (`RestartPolicy`)

This crate executes work safely and concurrently.

## Client -> stack trace

### Client code (what the framework user writes)

```rust
use actor_model::{
    RestartPolicy,
    spawn_counter,
    spawn_echo_agent,
    spawn_group_manager,
};

// 1) Counter actor
let counter = spawn_counter(8, RestartPolicy::Never).await?;
let value = counter.add(5).await?;
println!("counter = {value}");

// 2) Multi-agent group
let alice = spawn_echo_agent("alice", 8, RestartPolicy::Never).await?;
let bob = spawn_echo_agent("bob", 8, RestartPolicy::Never).await?;
let mgr = spawn_group_manager(vec![alice, bob], 8, RestartPolicy::Never).await?;
let transcript = mgr.run("hello", 4).await?;
println!("{:?}", transcript);
```

### Stack trace (what happens internally)

```CounterHandle::add -> HandleCore::request -> HandleCore::send -> SupervisorHandle::sender -> mpsc send -> actor recv loop -> handler updates state -> oneshot reply -> HandleCore::request returns```

Restart path:

```actor panic -> Supervisor::monitor_loop catches JoinError::panic -> allows_restart(policy) -> factory() respawn -> sender swap -> future sends go to new actor```

## API quick map

- `spawn_actor(capacity, initial_state, handler)`: generic actor primitive
- `ActorError`: mailbox, timeout, send, response, init errors
- `SendPolicy`: `Backpressure` vs `FailFast`
- `RestartPolicy`: `Never` or `MaxRetries { n }`
- Sample handles:
  - `CounterHandle` (`add`, `get`, `stop`, timeout variants)
  - `EchoAgentHandle` (`respond`, `stop`)
  - `GroupManagerHandle` (`run`, `stop`)

## Design note

`actor_model` focuses on execution mechanics.
Scheduling/planning concerns (DAG order, conditional enablement, passes) belong to `fsm_dag` and are typically composed by `workflow-orchestrator`.

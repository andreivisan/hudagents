use std::{collections::VecDeque, usize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State { Locked, Unlocked }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event { Coin, Push }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action { Unlock, Lock, Alarm, ThankYou, EnqueueCoin, EnqueuePush}

#[derive(Debug, Default, PartialEq, Eq)]
struct Ctx {
    coins: u32,
    pushes: u32,
    alarms: u32,
}

fn step(state: State, event: Event, ctx: &mut Ctx) -> (State, Vec<Action>) {
    let mut actions = Vec::new();
    let state: State  = match (state, event) {
        (State::Locked, Event::Coin) => {
            ctx.coins += 1;
            actions.push(Action::Unlock);
            State::Unlocked
        },
        (State::Locked, Event::Push) => {
            ctx.alarms += 1;
            ctx.pushes += 1;
            return (State::Locked, vec![Action::Alarm, Action::EnqueueCoin])
        },
        (State::Unlocked, Event::Coin) => {
            ctx.coins += 1;
            actions.push(Action::ThankYou);
            State::Unlocked
        },
        (State::Unlocked, Event::Push) => {
            ctx.pushes += 1;
            actions.push(Action::Lock);
            State::Locked
        }
    };
    (state, actions)
}

fn run(init_state: State, init_events: &[Event], ctx: &mut Ctx, max_steps: usize) -> (State, Vec<Action>, bool) {
    let mut q: VecDeque<Event> = init_events.iter().copied().collect(); 
    let mut state = init_state;
    let mut step_counter: usize = 0;
    let mut all_actions: Vec<Action> = Vec::new();
    while step_counter < max_steps {
        let Some(event) = q.pop_front() else { break;} ;
        step_counter += 1;
        let (next_step, action_for_step) = step(state, event, ctx);
        for action in &action_for_step {
            match action {
                Action::EnqueueCoin => q.push_back(Event::Coin),
                Action::EnqueuePush => q.push_back(Event::Push),
                _ => {}
            }
        }
        all_actions.extend(action_for_step);
        state = next_step;
    }
    let hit_cap: bool = step_counter == max_steps && !q.is_empty();
    (state, all_actions, hit_cap) // last bool tells us if we hit max steps or not
}

fn kahn(num_nodes: usize, edges: &[(usize, usize)]) -> Vec<usize> {
    let mut result: Vec<usize> = Vec::with_capacity(num_nodes); 
    let mut in_degree = vec![0; num_nodes];
    let mut graph: Vec<Vec<usize>> = vec![Vec::new(); num_nodes]; 
    // compute each node's in-degree
    for &(origin, dest) in edges {
        debug_assert!(origin < num_nodes && dest < num_nodes);
        graph[origin].push(dest);
        in_degree[dest] += 1;
    }
    let mut q: VecDeque<usize> = VecDeque::new();
    for node in 0..num_nodes {
        if in_degree[node] == 0 { q.push_back(node) } 
    }
    let mut processed = 0usize;
    while let Some(node) = q.pop_front() {
        processed += 1;
        result.push(node);
        for &next_node in &graph[node] {
            in_degree[next_node] -= 1;
            if in_degree[next_node] == 0 { q.push_back(next_node); }
        }
    }
    if processed < num_nodes { vec![] }
    else { result }
}

fn run_passes(
  init_state: State,
  num_nodes: usize,
  edges: &[(usize, usize)],
  max_passes: usize,
  max_steps_per_pass: usize,
) -> (State, Ctx, Vec<Action>) {
    let mut state = init_state;
    let mut ctx: Ctx = Ctx::default();
    let mut all_actions: Vec<Action> = Vec::new();
    let ordered: Vec<usize> = kahn(num_nodes, edges);
    if ordered.is_empty() { return (state, Ctx::default(), vec![]); }
    let mut events: Vec<Event> = Vec::new();
    for _ in 0..max_passes {
        events.clear();
        for &node in &ordered {
            match node {
                0 => {
                    if state == State::Locked { events.push(Event::Push); }
                },
                2 => {
                    if state == State::Locked {
                        events.push(Event::Coin);
                    } else if ctx.coins < 3 {
                        events.push(Event::Coin);
                    }
                },
                _ => {}
            }
        }
        let (next_step, next_actions, _) = run(state, &events, &mut ctx, max_steps_per_pass);   
        state = next_step;
        all_actions.extend(next_actions);
        if state == State::Unlocked { return (state, ctx, all_actions); }
    }
    (state, ctx, all_actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multiple_events() {
        let events = [Event::Push, Event::Coin, Event::Push, Event::Coin];
        let mut state = State::Locked;
        let mut actions: Vec<Action> = Vec::new();
        let mut ctx: Ctx = Ctx::default();
        for event in events {
            let (next_state, temp_acc) = step(state, event, &mut ctx);
            actions.extend(temp_acc);
            state = next_state;
        }
        assert_eq!(ctx, Ctx{ coins: 2, pushes: 2, alarms: 1 });
        assert_eq!(state, State::Unlocked);
        assert_eq!(actions, vec![Action::Alarm, Action::EnqueueCoin, Action::Unlock, Action::Lock, Action::Unlock]);
    }

    #[test]
    fn test_run() {
        let init_state = State::Locked;
        let init_events = [Event::Push];
        let mut ctx: Ctx = Ctx::default();
        let max_steps = 10;
        let (state, all_actions, hit_cap) = run(init_state, &init_events, &mut ctx, max_steps);
        assert_eq!(state, State::Unlocked);
        assert_eq!(ctx, Ctx{ coins: 1, pushes: 1, alarms: 1 });
        assert_eq!(all_actions, vec![Action::Alarm, Action::EnqueueCoin, Action::Unlock]);
        assert_eq!(hit_cap, false);
    }

    #[test]
    fn test_khan() {
        let result: Vec<usize>  = kahn(4, &[(0, 2), (1, 2), (2, 3)]);
        assert_eq!(result.len(), 4);
        assert_eq!(result, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_khan_cycle() {
        let result: Vec<usize>  = kahn(4, &[(0, 2), (1, 2), (2, 3), (3, 1)]);
        assert_eq!(result, vec![]);
    }

    #[test]
    fn test_run_passes() {
        let num_nodes = 4;
        let edges = [(0, 2), (1, 2), (2, 3)];
        let (state, _, actions) = run_passes(State::Locked, num_nodes, &edges, 10, 10);
        assert_eq!(state, State::Unlocked);
        assert!(actions.iter().any(|a| matches!(a, Action::Unlock)));
    }

    #[test]
    fn test_run_passes_unlocks_once_and_stops_pushing() {
        let num_nodes = 4;
        let edges = [(0, 2), (1, 2), (2, 3)];
        let (state, ctx, _) = run_passes(State::Locked, num_nodes, &edges, 8, 10);
        assert_eq!(state, State::Unlocked);
        assert_eq!(ctx.pushes, 1);
        assert!(ctx.coins <= 3);
    }

}

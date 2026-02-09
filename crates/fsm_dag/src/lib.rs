use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub enum Cond<A> {
    Always,
    Atom(A),
    And(Box<Cond<A>>, Box<Cond<A>>),
    Or(Box<Cond<A>>, Box<Cond<A>>),
    Not(Box<Cond<A>>),
}

pub trait AtomEval<S, C> {
    fn eval(&self, state: &S, ctx: &C) -> bool;
}

impl<A> Cond<A> {
    pub fn eval<S, C>(&self, state: &S, ctx: &C) -> bool
    where
        A: AtomEval<S, C>,
    {
        match self {
            Cond::Always => true,
            Cond::Atom(a) => a.eval(state, ctx),
            Cond::And(x, y) => x.eval(state, ctx) && y.eval(state, ctx),
            Cond::Or(x, y) => x.eval(state, ctx) || y.eval(state, ctx),
            Cond::Not(x) => !x.eval(state, ctx),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    CycleDetected,
    NoProgress,
    HitMaxPasses,
    ExhaustedPassViews,
    InvalidInput,
}

#[derive(Clone, Copy, Debug)]
pub struct PassView<'state, 'ctx, S, C> {
    pub state: &'state S,
    pub ctx: &'ctx C,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassPlan {
    pub topo_order: Vec<usize>,
    pub enabled_per_pass: Vec<Vec<usize>>,
    pub stop_reason: StopReason,
}

pub fn run<S, C, A>(state: &S, ctx: &C, topo_order: &[usize], enabled: &[Cond<A>]) -> Vec<usize>
where
    A: AtomEval<S, C>,
{
    let mut runnable = Vec::with_capacity(topo_order.len());
    for &node in topo_order {
        if let Some(cond) = enabled.get(node)
            && cond.eval(state, ctx)
        {
            runnable.push(node);
        }
    }
    runnable
}

pub fn kahn(num_nodes: usize, edges: &[(usize, usize)]) -> Vec<usize> {
    let mut result: Vec<usize> = Vec::with_capacity(num_nodes);
    let mut in_degree = vec![0; num_nodes];
    let mut graph: Vec<Vec<usize>> = vec![Vec::new(); num_nodes];

    for &(origin, dest) in edges {
        if origin >= num_nodes || dest >= num_nodes {
            return vec![];
        }
        graph[origin].push(dest);
        in_degree[dest] += 1;
    }

    let mut q: VecDeque<usize> = VecDeque::new();
    for (node, degree) in in_degree.iter().enumerate().take(num_nodes) {
        if *degree == 0 {
            q.push_back(node);
        }
    }

    let mut processed = 0usize;
    while let Some(node) = q.pop_front() {
        processed += 1;
        result.push(node);
        for &next_node in &graph[node] {
            in_degree[next_node] -= 1;
            if in_degree[next_node] == 0 {
                q.push_back(next_node);
            }
        }
    }

    if processed < num_nodes {
        vec![]
    } else {
        result
    }
}

pub fn run_passes<S, C, A>(
    num_nodes: usize,
    edges: &[(usize, usize)],
    enabled: &[Cond<A>],
    pass_views: &[PassView<'_, '_, S, C>],
    max_passes: usize,
) -> PassPlan
where
    A: AtomEval<S, C>,
{
    if enabled.len() != num_nodes {
        return PassPlan {
            topo_order: Vec::new(),
            enabled_per_pass: Vec::new(),
            stop_reason: StopReason::InvalidInput,
        };
    }

    if edges
        .iter()
        .any(|&(origin, dest)| origin >= num_nodes || dest >= num_nodes)
    {
        return PassPlan {
            topo_order: Vec::new(),
            enabled_per_pass: Vec::new(),
            stop_reason: StopReason::InvalidInput,
        };
    }

    let ordered = kahn(num_nodes, edges);
    if ordered.is_empty() && num_nodes > 0 {
        return PassPlan {
            topo_order: ordered,
            enabled_per_pass: Vec::new(),
            stop_reason: StopReason::CycleDetected,
        };
    }

    let pass_limit = max_passes.min(pass_views.len());
    let mut enabled_per_pass: Vec<Vec<usize>> = Vec::with_capacity(pass_limit);

    for pass in pass_views.iter().take(pass_limit) {
        let runnable = run(pass.state, pass.ctx, &ordered, enabled);
        if runnable.is_empty() {
            return PassPlan {
                topo_order: ordered,
                enabled_per_pass,
                stop_reason: StopReason::NoProgress,
            };
        }
        enabled_per_pass.push(runnable);
    }

    let stop_reason = if pass_views.len() > pass_limit {
        StopReason::HitMaxPasses
    } else {
        StopReason::ExhaustedPassViews
    };

    PassPlan {
        topo_order: ordered,
        enabled_per_pass,
        stop_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum State {
        Locked,
        Unlocked,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Event {
        Coin,
        Push,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Action {
        Unlock,
        Lock,
        Alarm,
        ThankYou,
        EnqueueCoin,
        EnqueuePush,
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    struct Ctx {
        coins: u32,
        pushes: u32,
        alarms: u32,
    }

    #[derive(Clone, Debug)]
    enum CoinAtom {
        StateIs(State),
        CoinsLt(u32),
    }

    impl AtomEval<State, Ctx> for CoinAtom {
        fn eval(&self, state: &State, ctx: &Ctx) -> bool {
            match self {
                CoinAtom::StateIs(s) => state == s,
                CoinAtom::CoinsLt(n) => ctx.coins < *n,
            }
        }
    }

    fn coin_enabled() -> Vec<Cond<CoinAtom>> {
        vec![
            Cond::Atom(CoinAtom::StateIs(State::Locked)),
            Cond::Always,
            Cond::Or(
                Box::new(Cond::Atom(CoinAtom::StateIs(State::Locked))),
                Box::new(Cond::Atom(CoinAtom::CoinsLt(3))),
            ),
            Cond::Atom(CoinAtom::StateIs(State::Unlocked)),
        ]
    }

    fn step(state: State, event: Event, ctx: &mut Ctx) -> (State, Vec<Action>) {
        let mut actions = Vec::new();
        let state = match (state, event) {
            (State::Locked, Event::Coin) => {
                ctx.coins += 1;
                actions.push(Action::Unlock);
                State::Unlocked
            }
            (State::Locked, Event::Push) => {
                ctx.alarms += 1;
                ctx.pushes += 1;
                return (State::Locked, vec![Action::Alarm, Action::EnqueueCoin]);
            }
            (State::Unlocked, Event::Coin) => {
                ctx.coins += 1;
                actions.push(Action::ThankYou);
                State::Unlocked
            }
            (State::Unlocked, Event::Push) => {
                ctx.pushes += 1;
                actions.push(Action::Lock);
                State::Locked
            }
        };
        (state, actions)
    }

    fn run_event_queue(
        init_state: State,
        init_events: &[Event],
        ctx: &mut Ctx,
        max_steps: usize,
    ) -> (State, Vec<Action>, bool) {
        let mut q: VecDeque<Event> = init_events.iter().copied().collect();
        let mut state = init_state;
        let mut step_counter = 0usize;
        let mut all_actions: Vec<Action> = Vec::new();

        while step_counter < max_steps {
            let Some(event) = q.pop_front() else {
                break;
            };
            step_counter += 1;
            let (next_state, actions_for_step) = step(state, event, ctx);

            for action in &actions_for_step {
                match action {
                    Action::EnqueueCoin => q.push_back(Event::Coin),
                    Action::EnqueuePush => q.push_back(Event::Push),
                    _ => {}
                }
            }

            all_actions.extend(actions_for_step);
            state = next_state;
        }

        let hit_cap = step_counter == max_steps && !q.is_empty();
        (state, all_actions, hit_cap)
    }

    #[test]
    fn test_multiple_events() {
        let _ = Action::EnqueuePush;
        let events = [Event::Push, Event::Coin, Event::Push, Event::Coin];
        let mut state = State::Locked;
        let mut actions: Vec<Action> = Vec::new();
        let mut ctx: Ctx = Ctx::default();
        for event in events {
            let (next_state, temp_acc) = step(state, event, &mut ctx);
            actions.extend(temp_acc);
            state = next_state;
        }
        assert_eq!(
            ctx,
            Ctx {
                coins: 2,
                pushes: 2,
                alarms: 1
            }
        );
        assert_eq!(state, State::Unlocked);
        assert_eq!(
            actions,
            vec![
                Action::Alarm,
                Action::EnqueueCoin,
                Action::Unlock,
                Action::Lock,
                Action::Unlock
            ]
        );
    }

    #[test]
    fn test_run_event_queue() {
        let init_state = State::Locked;
        let init_events = [Event::Push];
        let mut ctx: Ctx = Ctx::default();
        let max_steps = 10;
        let (state, all_actions, hit_cap) =
            run_event_queue(init_state, &init_events, &mut ctx, max_steps);
        assert_eq!(state, State::Unlocked);
        assert_eq!(
            ctx,
            Ctx {
                coins: 1,
                pushes: 1,
                alarms: 1
            }
        );
        assert_eq!(
            all_actions,
            vec![Action::Alarm, Action::EnqueueCoin, Action::Unlock]
        );
        assert!(!hit_cap);
    }

    #[test]
    fn test_khan() {
        let result: Vec<usize> = kahn(4, &[(0, 2), (1, 2), (2, 3)]);
        assert_eq!(result.len(), 4);
        assert_eq!(result, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_khan_cycle() {
        let result: Vec<usize> = kahn(4, &[(0, 2), (1, 2), (2, 3), (3, 1)]);
        assert_eq!(result, vec![]);
    }

    #[test]
    fn test_run_returns_enabled_nodes_in_topological_order() {
        let state = State::Locked;
        let ctx = Ctx::default();
        let topo_order = vec![0, 1, 2, 3];
        let enabled = coin_enabled();

        let runnable = run(&state, &ctx, &topo_order, &enabled);
        assert_eq!(runnable, vec![0, 1, 2]);
    }

    #[test]
    fn test_run_passes() {
        let num_nodes = 4;
        let edges = [(0, 2), (1, 2), (2, 3)];
        let enabled = coin_enabled();
        let s0 = State::Locked;
        let c0 = Ctx {
            coins: 0,
            pushes: 0,
            alarms: 0,
        };
        let s1 = State::Unlocked;
        let c1 = Ctx {
            coins: 1,
            pushes: 1,
            alarms: 0,
        };
        let s2 = State::Unlocked;
        let c2 = Ctx {
            coins: 3,
            pushes: 1,
            alarms: 0,
        };

        let pass_views = vec![
            PassView {
                state: &s0,
                ctx: &c0,
            },
            PassView {
                state: &s1,
                ctx: &c1,
            },
            PassView {
                state: &s2,
                ctx: &c2,
            },
        ];

        let plan = run_passes(num_nodes, &edges, &enabled, &pass_views, 10);
        assert_eq!(plan.topo_order, vec![0, 1, 2, 3]);
        assert_eq!(
            plan.enabled_per_pass,
            vec![vec![0, 1, 2], vec![1, 2, 3], vec![1, 3]]
        );
        assert_eq!(plan.stop_reason, StopReason::ExhaustedPassViews);
    }

    #[test]
    fn test_run_passes_unlocks_once_and_stops_pushing() {
        let num_nodes = 4;
        let edges = [(0, 2), (1, 2), (2, 3)];
        let enabled = coin_enabled();

        let s0 = State::Locked;
        let c0 = Ctx {
            coins: 0,
            pushes: 0,
            alarms: 0,
        };
        let s1 = State::Unlocked;
        let c1 = Ctx {
            coins: 1,
            pushes: 1,
            alarms: 0,
        };
        let s2 = State::Unlocked;
        let c2 = Ctx {
            coins: 2,
            pushes: 1,
            alarms: 0,
        };

        let pass_views = vec![
            PassView {
                state: &s0,
                ctx: &c0,
            },
            PassView {
                state: &s1,
                ctx: &c1,
            },
            PassView {
                state: &s2,
                ctx: &c2,
            },
        ];

        let plan = run_passes(num_nodes, &edges, &enabled, &pass_views, 2);
        assert_eq!(plan.enabled_per_pass, vec![vec![0, 1, 2], vec![1, 2, 3]]);
        assert_eq!(plan.stop_reason, StopReason::HitMaxPasses);
    }

    #[test]
    fn test_run_conditional_edges() {
        let num_nodes = 4;
        let edges = [(0, 2), (1, 2), (2, 3)];
        let enabled: Vec<Cond<CoinAtom>> = vec![
            Cond::Atom(CoinAtom::StateIs(State::Locked)),
            Cond::Atom(CoinAtom::StateIs(State::Locked)),
            Cond::Atom(CoinAtom::StateIs(State::Locked)),
            Cond::Atom(CoinAtom::StateIs(State::Locked)),
        ];
        let s0 = State::Unlocked;
        let c0 = Ctx {
            coins: 2,
            pushes: 1,
            alarms: 0,
        };
        let pass_views = vec![PassView {
            state: &s0,
            ctx: &c0,
        }];

        let plan = run_passes(num_nodes, &edges, &enabled, &pass_views, 8);
        assert_eq!(plan.enabled_per_pass, Vec::<Vec<usize>>::new());
        assert_eq!(plan.stop_reason, StopReason::NoProgress);
    }

    #[test]
    fn test_run_passes_cycle_detected() {
        let num_nodes = 4;
        let edges = [(0, 2), (1, 2), (2, 3), (3, 1)];
        let enabled = coin_enabled();
        let s0 = State::Locked;
        let c0 = Ctx::default();
        let pass_views = vec![PassView {
            state: &s0,
            ctx: &c0,
        }];

        let plan = run_passes(num_nodes, &edges, &enabled, &pass_views, 10);
        assert_eq!(plan.topo_order, Vec::<usize>::new());
        assert_eq!(plan.stop_reason, StopReason::CycleDetected);
    }

    #[test]
    fn test_run_passes_invalid_input() {
        let num_nodes = 4;
        let edges = [(0, 2), (1, 2), (2, 3)];
        let enabled = vec![Cond::<CoinAtom>::Always, Cond::Always];
        let s0 = State::Locked;
        let c0 = Ctx::default();
        let pass_views = vec![PassView {
            state: &s0,
            ctx: &c0,
        }];

        let plan = run_passes(num_nodes, &edges, &enabled, &pass_views, 10);
        assert_eq!(plan.stop_reason, StopReason::InvalidInput);
    }
}

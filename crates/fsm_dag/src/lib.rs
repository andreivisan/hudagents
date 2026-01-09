use std::collections::VecDeque;

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

fn run(init_state: State, init_events: Vec<Event>, max_steps: usize) -> (State, Ctx, Vec<Action>, bool) {
    let mut q: VecDeque<Event> = init_events.into(); 
    let mut state = init_state;
    let mut step_counter: usize = 0;
    let mut all_actions: Vec<Action> = Vec::new();
    let mut ctx: Ctx = Ctx::default();
    while step_counter < max_steps {
        step_counter += 1;
        let Some(event) = q.pop_front() else { break;} ;
        let (next_step, action_for_step) = step(init_state, event, &mut ctx);
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
    (state, ctx, all_actions, !q.is_empty()) // last bool tells us if we hit max steps or not
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multiple_events() {
        // Create a vec of events
        let events = vec![Event::Push, Event::Coin, Event::Push, Event::Coin];
        let mut state = State::Locked;
        let mut actions: Vec<Action> = Vec::new();
        let mut ctx: Ctx = Ctx::default();
        // Loop through the vec
        for event in events {
            let (next_state, temp_acc) = step(state, event, &mut ctx);
            // Append actions for each event to an outer accumulator
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
        let init_events = vec![Event::Push];
        let max_steps = 10;
        let (state, ctx, all_actions, hit_cap) = run(init_state, init_events, max_steps);
        assert_eq!(state, State::Unlocked);
        assert_eq!(ctx, Ctx{ coins: 1, pushes: 1, alarms: 1 });
        assert_eq!(all_actions, vec![Action::Alarm, Action::EnqueueCoin, Action::Unlock]);
        assert_eq!(hit_cap, false);
    }

}


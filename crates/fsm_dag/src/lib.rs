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
            return (State::Locked, vec![Action::Alarm])
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

fn run(init_state: State, init_events: Vec<Event>, max_steps: usize) -> (State, Ctx, Vec<Action>) {}

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
        assert_eq!(actions, vec![Action::Alarm, Action::Unlock, Action::Lock, Action::Unlock]);
    }

}


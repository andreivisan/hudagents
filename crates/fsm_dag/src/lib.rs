#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State { Locked, Unlocked }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event { Coin, Push }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action { Unlock, Lock, Alarm, ThankYou, NoOp }

#[derive(Debug, Default)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multiple_events() {
        // Create a queue of events
        // Loop through the queue
        // Append actions for each event to an outer accumulator
    }

}


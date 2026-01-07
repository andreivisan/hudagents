#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State { Locked, Unlocked }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event { Coin, Push }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action { Unlock, Lock, Alarm, ThankYou, NoOp }

fn step(state: State, event: Event) -> (State, Action) {
    match (state, event) {
        (State::Locked, Event::Coin) => (State::Unlocked, Action::Unlock),
        (State::Locked, Event::Push) => (State::Locked, Action::Alarm),
        (State::Unlocked, Event::Coin) => (State::Unlocked, Action::ThankYou),
        (State::Unlocked, Event::Push) => (State::Locked, Action::Lock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locked_coin() {
        let (state, action) = step(State::Locked, Event::Coin);
        assert_eq!(State::Unlocked, state);
        assert_eq!(Action::Unlock, action);
    }    

    #[test]
    fn test_locked_push() {
        let (state, action) = step(State::Locked, Event::Push);
        assert_eq!(State::Locked, state);
        assert_eq!(Action::Alarm, action);
    }

    #[test]
    fn test_unlocked_coin() {
        let (state, action) = step(State::Unlocked, Event::Coin);
        assert_eq!(State::Unlocked, state);
        assert_eq!(Action::ThankYou, action);
    }

    #[test]
    fn test_unlocked_push() {
        let (state, action) = step(State::Unlocked, Event::Push);
        assert_eq!(State::Locked, state);
        assert_eq!(Action::Lock, action);
    }
}


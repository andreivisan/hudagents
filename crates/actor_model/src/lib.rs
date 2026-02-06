// ************************************************************************* //
// ********************* Summary in one line ******************************* //
// add → request_with_timeout → request → supervisor.sender() → mpsc send → 
// actor recv → handler updates state → oneshot reply → back up stack.
// ************************************************************************* //

use std::{future::Future, sync::Arc};
use tokio::{
    sync::{
        mpsc::{error::TrySendError, channel, Sender},
        oneshot::{self, Sender as ReplyTx},
        RwLock,
    },
    task::JoinHandle,
    time::{sleep, timeout, Duration},
};

// ************************************************************************** //
// ******************* ACTOR MODEL FRAMEWORK ******* ************************ //
// ************************************************************************** //

#[derive(Debug)]
pub enum ActorError {
    InitError,
    InvalidCapacity,
    MailboxFull,
    ResponseDropped,
    SendFailed,
    Timeout,
}

#[derive(Debug)]
pub enum ActorCtrl {
    Continue,
    Stop,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExitReason {
    StoppedByMessage,
    AllSendersDropped,
}

#[derive(Clone, Copy, Debug)]
pub enum RestartPolicy {
    MaxRetries { n: usize },
    Never,
}


#[derive(Debug)]
pub enum Termination {
    Clean(ExitReason),
    Panic,
}

#[derive(Clone, Copy, Debug)]
pub enum SendPolicy {
    Backpressure,
    FailFast,
}

// Helper function
fn allows_restart(policy: RestartPolicy, attempts_so_far: usize) -> bool {
    match policy {
        RestartPolicy::MaxRetries{ n } => attempts_so_far < n,
        RestartPolicy::Never => { false }
    }
}

struct Supervisor<Msg, F> 
where
    Msg: Send + 'static,
    F: Fn() -> Result<(Sender<Msg>, JoinHandle<ExitReason>), ActorError> + Send + Sync + 'static,
{
    factory: F,
    policy: RestartPolicy,
    monitor_join: JoinHandle<()>,
    restart_attempts: usize,
    sender_slot: Arc<RwLock<Sender<Msg>>>,
}

impl<Msg, F> Supervisor<Msg, F>
where
    Msg: Send + 'static,
    F: Fn() -> Result<(Sender<Msg>, JoinHandle<ExitReason>), ActorError> + Send + Sync + 'static,
{
    async fn monitor_loop(
        factory: F, 
        policy: RestartPolicy, 
        sender_slot: Arc<RwLock<Sender<Msg>>>,
        mut join: JoinHandle<ExitReason>
    ) {
        let mut attempts = 0;
        loop {
            match join.await {
                Ok(exit) => { println!("join ok: exit reason {:?}", exit); return; }
                Err(err) => {
                    if !err.is_panic() { return; }
                    if !allows_restart(policy, attempts) { return; } 
                    attempts += 1;
                    println!("Restart attempt no. {}", attempts);
                    let (new_tx, new_join) = match factory() {
                        Ok(v) => v,
                        Err(_) => return,
                    };
                    // swap sender
                    {
                        let mut slot = sender_slot.write().await;
                        *slot = new_tx;
                    }
                    println!("sender swapped");
                    join = new_join;
                }
            }
       }
    }

    async fn start(factory: F, policy: RestartPolicy) -> Result<SupervisorHandle<Msg>, ActorError> {
        let (tx, join) = (factory)()?;
        let sender_slot = Arc::new(RwLock::new(tx));
        tokio::spawn(Self::monitor_loop(factory, policy, sender_slot.clone(), join)); 
        Ok(SupervisorHandle { sender_slot })    
    }
}

struct SupervisorHandle<Msg>
where
    Msg: Send + 'static,
{
    sender_slot: Arc<RwLock<Sender<Msg>>>,
}

impl<Msg> SupervisorHandle<Msg> 
where
    Msg: Send + 'static
{
    async fn sender(&self) -> Sender<Msg> {
        let guard = self.sender_slot.read().await;
        let sender = guard.clone();
        drop(guard);
        sender
    }
}

impl<Msg> Clone for SupervisorHandle<Msg>
where
    Msg: Send + 'static
{
    fn clone(&self) -> Self {
        Self {
            sender_slot: self.sender_slot.clone(),           
        }
    }
}

pub fn spawn_actor<State, Msg, Handler, Fut>(
    capacity: usize,
    initial_state: State,
    handler: Handler
) -> Result<(Sender<Msg>, JoinHandle<ExitReason>), ActorError>
where
    State: Send + 'static,
    Msg: Send + 'static,
    Handler: FnMut(&mut State, Msg) -> Fut + Send + 'static,
    Fut: Future<Output = ActorCtrl> + Send + 'static
{
    if capacity == 0 { return Err(ActorError::InvalidCapacity) }
    let (tx, mut rx) = channel::<Msg>(capacity);
    let join = tokio::spawn(async move {
        let mut state = initial_state;
        let mut handler = handler;
        println!("Actor started");
        while let Some(msg) = rx.recv().await {
            println!("Actor got message");
            match handler(&mut state, msg).await {
                ActorCtrl::Continue => {}
                ActorCtrl::Stop => return ExitReason::StoppedByMessage,
            }
        }
        ExitReason::AllSendersDropped
    });
    Ok((tx, join))
}

// ************************************************************************** //
// ******************* COUNTER ONE AGENT SIMMULATION ************************ //
// ************************************************************************** //

// Default timeout for actor requests (5s).
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(200);

enum CounterMessage {
    Add { delta: i64, reply: ReplyTx<i64> },
    Get { reply: ReplyTx<i64> },
    DelayGet { delay: Duration, reply: ReplyTx<i64> },
    Stop { reply: ReplyTx<()> },
    CrashNow { reply: ReplyTx<()> },
    #[cfg(test)]
    Hold { started: oneshot::Sender<()>, release: oneshot::Receiver<()> },
}

#[derive(Clone)]
pub struct CounterHandle {
    sup: SupervisorHandle<CounterMessage>,
    default_timeout: Duration,
    send_policy: SendPolicy,
}

impl CounterHandle {

    pub fn with_policy(&self, policy: SendPolicy) -> Self {
        Self {
            sup: self.sup.clone(),
            default_timeout: self.default_timeout.clone(),
            send_policy: policy,
        }
    }

    async fn request<T>(
        &self, 
        make_msg: impl FnOnce(ReplyTx<T>) -> CounterMessage
    ) -> Result<T, ActorError> {
        let (reply_tx,  reply_rx) = oneshot::channel();
        let msg = make_msg(reply_tx);
        let sender = self.sup.sender().await;
        println!("handle send");
        match self.send_policy {
            SendPolicy::FailFast => {
                match sender.try_send(msg) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => return Err(ActorError::MailboxFull),
                    Err(TrySendError::Closed(_)) => return Err(ActorError::SendFailed),
                }
            }
            SendPolicy::Backpressure => sender.send(msg).await.map_err(|_| ActorError::SendFailed)?,
        }
        println!("handle sent");
        reply_rx.await.map_err(|_| ActorError::ResponseDropped)
    } 

    async fn request_with_timeout<T>(
        &self, 
        timeout_opt: Option<Duration>,
        make_msg: impl FnOnce(ReplyTx<T>) -> CounterMessage
    ) -> Result<T, ActorError> {
        let fut = self.request(make_msg);
        let effective_timeout = timeout_opt.unwrap_or(self.default_timeout);
        match timeout(effective_timeout, fut).await {
            Ok(res) => { println!("reply ok"); res },
            Err(_) => { println!("handle timeout"); Err(ActorError::Timeout) },
        }
    }

    pub async fn add(&self, delta: i64) -> Result<i64, ActorError> {
        self.request_with_timeout(None, |reply| CounterMessage::Add { delta, reply }).await
    }

    pub async fn add_with_timeout(&self, delta: i64, duration: Duration) -> Result<i64, ActorError> {
        self.request_with_timeout(Some(duration), |reply| CounterMessage::Add { delta, reply }).await
    }

    pub async fn get(&self) -> Result<i64, ActorError> {
        self.request_with_timeout(None, |reply| CounterMessage::Get { reply }).await
    }

    pub async fn get_with_timeout(&self, duration: Duration) -> Result<i64, ActorError> {
        self.request_with_timeout(Some(duration), |reply| CounterMessage::Get { reply }).await
    }

    pub async fn stop(&self) -> Result<(), ActorError> {
        self.request_with_timeout(None, |reply| CounterMessage::Stop { reply }).await
    }

    pub async fn stop_with_timeout(&self, duration: Duration) -> Result<(), ActorError> {
        self.request_with_timeout(Some(duration), |reply| CounterMessage::Stop { reply }).await
    }

    pub async fn crash_now(&self) -> Result<(), ActorError> {
        self.request_with_timeout(None, |reply| CounterMessage::CrashNow { reply }).await
    }
}

pub async fn spawn_counter(capacity: usize, policy: RestartPolicy) -> Result<CounterHandle, ActorError> {
    let factory = move || {
        spawn_actor(
            capacity,
            0_i64,
            |state, msg| {
                let mut delayed: Option<(Duration, ReplyTx<i64>, i64)> = None;
                let mut hold: Option<(oneshot::Sender<()>, oneshot::Receiver<()>)> = None;

                let ctrl = match msg {
                    CounterMessage::Add { delta, reply } => {
                        *state += delta;
                        let _ = reply.send(*state);
                        println!("actor add + {} = {}", delta, *state);
                        ActorCtrl::Continue
                    }
                    CounterMessage::Get { reply } => {
                        let _ = reply.send(*state);
                        println!("actor get = {}", *state);
                        ActorCtrl::Continue
                    }
                    CounterMessage::DelayGet { delay, reply } => {
                        let value = *state;
                        delayed = Some((delay, reply, value));
                        println!("actor delay_get delay = {:?}", delay.as_nanos());
                        ActorCtrl::Continue
                    }
                    CounterMessage::Stop { reply } => {
                        let _ = reply.send(());
                        println!("actor stop");
                        ActorCtrl::Stop
                    }
                    CounterMessage::CrashNow { reply } => {
                        let _ = reply.send(());
                        println!("actor crash now");
                        panic!("crash requested");
                    }
                    #[cfg(test)]
                    CounterMessage::Hold { started, release } => {
                        hold = Some((started, release));
                        ActorCtrl::Continue
                    }
                };

                async move {
                    if let Some((started, release)) = hold {
                        let _ = started.send(());
                        let _ = release.await;
                    }
                    if let Some((delay, reply, value)) = delayed {
                        sleep(delay).await;
                        let _ = reply.send(value);
                    }
                    ctrl
                }
            },
        )
    };
    let sup = Supervisor::start(factory, policy).await?;
    Ok(CounterHandle { sup, default_timeout: DEFAULT_TIMEOUT, send_policy: SendPolicy::Backpressure })
}

// ************************************************************************** //
// ******************* ECHO MULTI AGENT SIMMULATION ************************* //
// ************************************************************************** //

pub struct EchoState { name: String, turns: u64 }

enum EchoAgentMsg {
    Respond { input: String, reply: oneshot::Sender<String> },
    Stop { reply: oneshot::Sender<()> },
}

#[derive(Clone)]
pub struct EchoAgentHandle {
    sup: SupervisorHandle<EchoAgentMsg>,
    default_timeout: Duration,
    send_policy: SendPolicy,
}

impl EchoAgentHandle {

    pub async fn respond(&self, input: impl Into<String>) -> Result<String, ActorError> {
        let input = input.into();
        let (reply_tx, reply_rx) = oneshot::channel();
        let msg = EchoAgentMsg::Respond { input, reply: reply_tx };
        let sender = self.sup.sender().await;
        println!("Handle send");
        match self.send_policy {
            SendPolicy::FailFast => {
                match sender.try_send(msg) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => return Err(ActorError::MailboxFull),
                    Err(TrySendError::Closed(_)) => return Err(ActorError::SendFailed),
                }
            }
            SendPolicy::Backpressure => sender.send(msg).await.map_err(|_| ActorError::SendFailed)?,
        }
        println!("handle sent");
        match timeout(self.default_timeout, reply_rx).await {
            Ok(Ok(res)) => { println!("reply ok"); Ok(res) },
            Ok(Err(_)) => Err(ActorError::ResponseDropped),
            Err(_) => Err(ActorError::Timeout),
        }
    }

    pub async fn stop(&self) -> Result<(), ActorError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let msg = EchoAgentMsg::Stop { reply: reply_tx };
        let sender = self.sup.sender().await;
        println!("Handle istop send");
        match self.send_policy {
            SendPolicy::FailFast => {
                match sender.try_send(msg) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => return Err(ActorError::MailboxFull),
                    Err(TrySendError::Closed(_)) => return Err(ActorError::SendFailed),
                }
            }
            SendPolicy::Backpressure => sender.send(msg).await.map_err(|_| ActorError::SendFailed)?,
        }
        println!("handle sent");
        match timeout(self.default_timeout, reply_rx).await {
            Ok(Ok(())) => { println!("reply ok"); Ok(()) },
            Ok(Err(_)) => Err(ActorError::ResponseDropped),
            Err(_) => Err(ActorError::Timeout),
        }
    }

}

#[derive(Clone)]
pub struct GroupManagerState {
    agents: Vec<EchoAgentHandle>,
    next_idx: usize,
}

enum ManagerMsg {
    Run { initial: String, max_turns: usize, reply: oneshot::Sender<Vec<String>> },
    Stop { reply: oneshot::Sender<()> }
}

pub struct GroupManagerHandle {
    sup: SupervisorHandle<ManagerMsg>,
    default_timeout: Duration,
    send_policy: SendPolicy,
}

impl GroupManagerHandle {
    pub async fn run(&self, initial: impl Into<String>, max_turns: usize) -> Result<Vec<String>, ActorError> {
        let initial = initial.into();
        let (reply_tx, reply_rx) = oneshot::channel();
        let mngr_msg = ManagerMsg::Run { initial, max_turns, reply: reply_tx };
        let sender = self.sup.sender().await;
        println!("handle send");
        match self.send_policy {
            SendPolicy::FailFast => {
                match sender.try_send(mngr_msg) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => return Err(ActorError::MailboxFull),
                    Err(TrySendError::Closed(_)) => return Err(ActorError::SendFailed),
                }
            }
            SendPolicy::Backpressure => sender.send(mngr_msg).await.map_err(|_| ActorError::SendFailed)?,
        }
        println!("handle sent");
        match timeout(self.default_timeout, reply_rx).await {
            Ok(Ok(outputs)) => { println!("reply ok"); Ok(outputs) },
            Ok(Err(_)) => Err(ActorError::ResponseDropped),
            Err(_) => Err(ActorError::Timeout),
        }
    }

    pub async fn stop(&self) -> Result<(), ActorError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let msg = ManagerMsg::Stop { reply: reply_tx };
        let sender = self.sup.sender().await;
        println!("Handle istop send");
        match self.send_policy {
            SendPolicy::FailFast => {
                match sender.try_send(msg) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => return Err(ActorError::MailboxFull),
                    Err(TrySendError::Closed(_)) => return Err(ActorError::SendFailed),
                }
            }
            SendPolicy::Backpressure => sender.send(msg).await.map_err(|_| ActorError::SendFailed)?,
        }
        println!("handle sent");
        match timeout(self.default_timeout, reply_rx).await {
            Ok(Ok(())) => { println!("reply ok"); Ok(()) },
            Ok(Err(_)) => Err(ActorError::ResponseDropped),
            Err(_) => Err(ActorError::Timeout),
        }
    }
}

pub async fn spawn_group_manager(agents: Vec<EchoAgentHandle>, capacity: usize, policy: RestartPolicy) -> Result<GroupManagerHandle, ActorError> {
    if agents.len() == 0 { return Err(ActorError::InitError); }  
    let factory = move || {
        let initial_state = GroupManagerState { agents: agents.clone(), next_idx: 0 };
        spawn_actor(capacity, initial_state, |state, msg| {
            enum Action {
                Run {
                    picked: Vec<EchoAgentHandle>,
                    initial: String,
                    reply: oneshot::Sender<Vec<String>>
                },
                Stop {
                    reply: oneshot::Sender<()> 
                },
            }

            let action = match msg {
                ManagerMsg::Run { initial, max_turns, reply } => {
                    let len = state.agents.len();
                    let mut picked = Vec::with_capacity(max_turns);
                    for _ in 0..max_turns {
                        let idx = state.next_idx;
                        picked.push(state.agents[idx].clone());
                        state.next_idx = (idx + 1) % len;
                    }
                    Action::Run { picked, initial, reply }
                } 
                ManagerMsg::Stop { reply } => Action::Stop { reply },
            };

            async move {
                match action {
                    Action::Run { picked, initial, reply } => {
                        let mut outputs = Vec::new();
                        let mut input = initial;
                        for agent in picked {
                            match agent.respond(input).await {
                                Ok(out) => {
                                    input = out.clone();
                                    outputs.push(out);
                                }
                                Err(_e) => {
                                    let _ = reply.send(outputs); 
                                    return ActorCtrl::Continue;
                                }
                            }
                        }
                        let _ = reply.send(outputs);
                        ActorCtrl::Continue
                    }
                    Action::Stop { reply } => {
                        let _ = reply.send(());
                        ActorCtrl::Stop
                    }
                }
            }
        })
    };
    let sup = Supervisor::start(factory, policy).await?;
    Ok(GroupManagerHandle { sup, default_timeout: DEFAULT_TIMEOUT, send_policy: SendPolicy::Backpressure })
} 

pub async fn spawn_echo_agent(name: &str, capacity: usize, policy: RestartPolicy) -> Result<EchoAgentHandle, ActorError> {
    let name = name.to_string();
    let factory = move || {
        let initial_echo_state = EchoState { name: name.clone(), turns: 0 };  
        spawn_actor(
            capacity,
            initial_echo_state,
            |state, msg| {
                let ctrl = match msg {
                    EchoAgentMsg::Respond { input, reply } => {
                        state.turns += 1;
                        let output = format!("{}[{}]: {}", state.name, state.turns, input);
                        let _ = reply.send(output);
                        ActorCtrl::Continue
                    }
                    EchoAgentMsg::Stop { reply } => {
                        let _ = reply.send(());
                        ActorCtrl::Stop
                    }
                };
                async move { ctrl }
            },
        )   
    };
    let sup = Supervisor::start(factory, policy).await?;
    Ok(EchoAgentHandle { sup, default_timeout: DEFAULT_TIMEOUT, send_policy: SendPolicy::Backpressure })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parallel_actor_activity() {
        let h1 = spawn_counter(8, RestartPolicy::Never).await.unwrap();
        let h2 = spawn_counter(8, RestartPolicy::Never).await.unwrap();

        let t1 = tokio::spawn(async move {
            let _ = h1.add(1).await;
            let _ = h1.add(2).await;
        });

        let t2 = tokio::spawn(async move {
            let _ = h2.add(10).await;
            let _ = h2.get().await;
        });

        let _ = tokio::join!(t1, t2);
    }

    #[tokio::test]
    async fn test_counter_add_get_happy_path() {
        let handle = spawn_counter(8, RestartPolicy::Never).await.unwrap();

        let v1 = handle.add(5).await.unwrap();
        assert_eq!(v1, 5);

        let v2 = handle.add(-2).await.unwrap();
        assert_eq!(v2, 3);

        let v3 = handle.get().await.unwrap();
        assert_eq!(v3, 3);

        let _ = handle.stop().await;
    }

    #[tokio::test]
    async fn test_counter_stop_joins() {
        let handle = spawn_counter(8, RestartPolicy::Never).await.unwrap();

        let stop_res = handle.stop().await;
        assert!(stop_res.is_ok());
    }

    #[tokio::test]
    async fn test_counter_handle_clone_works() {
        let handle = spawn_counter(8, RestartPolicy::Never).await.unwrap();
        let handle2 = handle.clone();

        let v1 = handle.add(2).await.unwrap();
        assert_eq!(v1, 2);

        let v2 = handle2.add(3).await.unwrap();
        assert_eq!(v2, 5);

        let v3 = handle.get().await.unwrap();
        assert_eq!(v3, 5);

        let v4 = handle2.add(-1).await.unwrap();   
        assert_eq!(v4, 4);

        let v5 = handle2.get().await.unwrap();
        assert_eq!(v5, 4);

        let _ = handle.stop().await;
    }

    #[tokio::test]
    async fn test_exits_with_stop_by_message() {
        let (tx, join) = spawn_actor(8, 0_i64, |state, msg: i32| {
            let _ = msg;
            *state += 1;
            std::future::ready(ActorCtrl::Stop)
        }).unwrap();
        tx.send(1).await.unwrap();
        drop(tx);
        let exit = join.await.unwrap();
        assert_eq!(exit, ExitReason::StoppedByMessage);
    }

    #[tokio::test]
    async fn test_actor_exit_reason_all_senders_dropped() {
        let (tx, join) = spawn_actor(8, (), |_, _msg: ()| std::future::ready(ActorCtrl::Continue)).unwrap();
        drop(tx);
        let exit = join.await.unwrap();
        assert_eq!(exit, ExitReason::AllSendersDropped);
    }
    
    #[tokio::test]
    async fn test_override_path_works() {
        let handle = spawn_counter(8, RestartPolicy::Never).await.unwrap();
        let v1 = handle.get_with_timeout(Duration::from_millis(50)).await.unwrap();
        assert_eq!(v1, 0);
        let _ = handle.stop().await;

    }

    #[tokio::test(start_paused = true)]
    async fn test_timeout_triggered() {
        let handle = spawn_counter(8, RestartPolicy::Never).await.unwrap();

        let fut = handle.request_with_timeout(
            Some(Duration::from_millis(10)),
            |reply| CounterMessage::DelayGet { delay: Duration::from_millis(50), reply },
        );

        tokio::time::advance(Duration::from_millis(11)).await;
        let res = fut.await;
        assert!(matches!(res, Err(ActorError::Timeout)));
    }

    #[tokio::test(start_paused = true)]
    async fn test_restarts_once_on_panic() {
        let handle = spawn_counter(8, RestartPolicy::MaxRetries { n: 1 }).await.unwrap();
        let _v1 = handle.add(10).await;
        let _v2 = handle.crash_now().await;
        
        let mut v3 = None;
        for _ in 0..10 {
            match handle.add(1).await {
                Ok(v) => { v3 = Some(v); break; }
                Err(ActorError::SendFailed) => tokio::task::yield_now().await,
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }

        assert_eq!(v3, Some(1));
    }

    #[tokio::test(start_paused = true)]
    async fn test_does_not_restart_on_panic_when_never() {
        let handle = spawn_counter(8, RestartPolicy::Never).await.unwrap();
        let _v1 = handle.add(10).await;
        let _v2 = handle.crash_now().await;
        
        let mut v3 = handle.add(1).await;
        assert!(matches!(v3, Err(ActorError::SendFailed)));
    }

    #[tokio::test(start_paused = true)]
    async fn test_stops_restarting_after_max_retries() {
        let handle = spawn_counter(8, RestartPolicy::MaxRetries { n: 1 }).await.unwrap();
        let _ = handle.crash_now().await;

        let mut v3 = None;
        for _ in 0..10 {
            match handle.add(1).await {
                Ok(v) => {
                    v3 = Some(v);
                    break;
                }
                Err(ActorError::SendFailed) => tokio::task::yield_now().await,
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }
        assert_eq!(v3, Some(1));

        let _ = handle.crash_now().await;

        let mut ok = false;
        for _ in 0..10 {
            match handle.add(1).await {
                Ok(_) => {
                    ok = true;
                    break;
                }
                Err(ActorError::SendFailed) => tokio::task::yield_now().await,
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }
        assert!(!ok, "unexpected restart after retries exhausted");
    }

    #[tokio::test(start_paused = true)]
    async fn test_ff_returns_mailboxfull_when_q_full() {
        let counter = spawn_counter(1, RestartPolicy::Never).await.unwrap();
        let handle = counter.with_policy(SendPolicy::FailFast);    
        // send Hold
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();

        let sender = counter.sup.sender().await; // tests can access private fields
        sender.send(CounterMessage::Hold { started: started_tx, release: release_rx }).await.unwrap();

        // wait until actor is blocked
        let _ = started_rx.await;

        // enqueue one request (fills queue)
        let (dummy_tx, _dummy_rx) = oneshot::channel();
        sender
            .send(CounterMessage::Add { delta: 1, reply: dummy_tx })
            .await
            .unwrap();

        // second request should fail fast
        let res = handle.add(1).await;
        assert!(matches!(res, Err(ActorError::MailboxFull)));

        // release actor so the test doesn’t hang
        let _ = release_tx.send(());
    }

    #[tokio::test(start_paused = true)]
    async fn test_backpressure_does_not_return_mailboxfull() {
        let counter = spawn_counter(1, RestartPolicy::Never).await.unwrap();
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let sender = counter.sup.sender().await; // tests can access private fields
        sender
            .send(CounterMessage::Hold { started: started_tx, release: release_rx })
            .await
            .unwrap();

        // Wait until actor is blocked
        let _ = started_rx.await;

        // Fill the queue deterministically (capacity = 1)
        let (dummy_tx, _dummy_rx) = oneshot::channel();
        sender
            .send(CounterMessage::Add { delta: 1, reply: dummy_tx })
            .await
            .unwrap();
        let h2 = counter.clone();
        let pending = tokio::spawn(async move {
            h2.add(1).await
        });
        let _ = release_tx.send(());
        let res = pending.await.unwrap();
        assert!(res.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn test_cloned_handles_policy_independence() {
        let counter = spawn_counter(1, RestartPolicy::Never).await.unwrap();

        // default handle = Backpressure
        let h2 = counter.with_policy(SendPolicy::FailFast);

        // Hold actor
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();

        let sender = counter.sup.sender().await;
        sender
            .send(CounterMessage::Hold { started: started_tx, release: release_rx })
            .await
            .unwrap();

        let _ = started_rx.await;

        // Fill queue deterministically
        let (dummy_tx, _dummy_rx) = oneshot::channel();
        sender
            .send(CounterMessage::Add { delta: 1, reply: dummy_tx })
            .await
            .unwrap();

        // Backpressure handle: will block
        let pending = tokio::spawn(async move {
            counter.add(1).await
        });

        // FailFast handle: should error immediately
        let res = h2.add(1).await;
        assert!(matches!(res, Err(ActorError::MailboxFull)));

        // Release actor so backpressure can complete
        let _ = release_tx.send(());
        let res_bp = pending.await.unwrap();
        assert!(res_bp.is_ok());
    }

    #[tokio::test]
    async fn test_round_robin_order_is_correct() {
        let alice = spawn_echo_agent("alice", 8, RestartPolicy::Never).await.unwrap();
        let bob = spawn_echo_agent("bob", 8, RestartPolicy::Never).await.unwrap();
        let mgr = spawn_group_manager(vec![alice, bob], 8, RestartPolicy::Never).await.unwrap();

        let transcript = mgr.run("hello", 4).await.unwrap();

        assert_eq!(transcript.len(), 4);
        assert!(transcript[0].starts_with("alice[1]:"));
        assert!(transcript[1].starts_with("bob[1]:"));
        assert!(transcript[2].starts_with("alice[2]:"));
        assert!(transcript[3].starts_with("bob[2]:"));
    }

    #[tokio::test]
    async fn test_chaining_correctness() {
        let alice = spawn_echo_agent("alice", 8, RestartPolicy::Never).await.unwrap();
        let bob = spawn_echo_agent("bob", 8, RestartPolicy::Never).await.unwrap();
        let mgr = spawn_group_manager(vec![alice, bob], 8, RestartPolicy::Never).await.unwrap();

        let transcript = mgr.run("hello", 4).await.unwrap();

        assert_eq!(transcript.len(), 4);
        assert!(transcript[1].ends_with(&transcript[0]));
        assert!(transcript[2].ends_with(&transcript[1]));
        assert!(transcript[3].ends_with(&transcript[2]));
    }

    #[tokio::test]
    async fn test_manager_stop_ends_manager() {
        let alice = spawn_echo_agent("alice", 8, RestartPolicy::Never).await.unwrap();
        let bob = spawn_echo_agent("bob", 8, RestartPolicy::Never).await.unwrap();
        let mgr = spawn_group_manager(vec![alice, bob], 8, RestartPolicy::Never).await.unwrap();

        let stop_res = mgr.stop().await;
        assert!(stop_res.is_ok());

        let run_res = mgr.run("hello", 2).await;
        assert!(matches!(
            run_res,
            Err(ActorError::SendFailed) | Err(ActorError::Timeout)
        ));
    }
}

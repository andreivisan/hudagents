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

// === Generic actor runtime ===
#[derive(Debug)]
pub enum ActorError {
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

// === Counter example using the runtime ===

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
}

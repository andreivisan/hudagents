use std::{future::Future, sync::Arc};
use tokio::{
    sync::{
        mpsc::{channel, Sender},
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
    SendFailed,
    ResponseDropped,
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
        let mut attemtps = 0;
        loop {
            match join.await {
                Ok(_exit) => { return; }
                Err(err) => {
                    if !err.is_panic() { return; }
                    attemtps += 1;
                    if !allows_restart(policy, attemtps) { return; } 
                    let (new_tx, new_join) = match factory() {
                        Ok(v) => v,
                        Err(_) => return,
                    };
                    // swap sender
                    {
                        let mut slot = sender_slot.write().await;
                        *slot = new_tx;
                    }
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
        while let Some(msg) = rx.recv().await {
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
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

enum CounterMessage {
    Add { delta: i64, reply: ReplyTx<i64> },
    Get { reply: ReplyTx<i64> },
    DelayGet { delay: Duration, reply: ReplyTx<i64> },
    Stop { reply: ReplyTx<()> },
    CrashNow { reply: ReplyTx<()> },
}

#[derive(Clone)]
pub struct CounterHandle {
    sup: SupervisorHandle<CounterMessage>,
    default_timeout: Duration,
}

impl CounterHandle {
    async fn request<T>(
        &self, 
        make_msg: impl FnOnce(ReplyTx<T>) -> CounterMessage
    ) -> Result<T, ActorError> {
        let (reply_tx,  reply_rx) = oneshot::channel();
        let msg = make_msg(reply_tx);
        let sender = self.sup.sender().await;
        sender.send(msg).await.map_err(|_| ActorError::SendFailed)?;
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
            Ok(res) => res,
            Err(_) => Err(ActorError::Timeout),
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

                let ctrl = match msg {
                    CounterMessage::Add { delta, reply } => {
                        *state += delta;
                        let _ = reply.send(*state);
                        ActorCtrl::Continue
                    }
                    CounterMessage::Get { reply } => {
                        let _ = reply.send(*state);
                        ActorCtrl::Continue
                    }
                    CounterMessage::DelayGet { delay, reply } => {
                        let value = *state;
                        delayed = Some((delay, reply, value));
                        ActorCtrl::Continue
                    }
                    CounterMessage::Stop { reply } => {
                        let _ = reply.send(());
                        ActorCtrl::Stop
                    }
                    CounterMessage::CrashNow { reply } => {
                        let _ = reply.send(());
                        panic!("crash requested");
                    }
                };

                async move {
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
    Ok(CounterHandle { sup, default_timeout: DEFAULT_TIMEOUT })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}


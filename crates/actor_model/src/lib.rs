use std::future::Future;
use tokio::{
    sync::{
        mpsc::{channel, Sender},
        oneshot::{self, Sender as ReplyTx},
    },
    task::JoinHandle
};

#[derive(Debug)]
pub enum ActorError {
    SendFailed,
    ResponseDropped,
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

enum Message {
    Add { delta: i64, reply: ReplyTx<i64> },
    Get { reply: ReplyTx<i64> },
    Stop { reply: ReplyTx<()> },
}

#[derive(Clone)]
pub struct CounterHandle {
    tx: Sender<Message>, 
}

impl CounterHandle {
    async fn request<T>(
        &self, 
        make_msg: impl FnOnce(ReplyTx<T>) -> Message
    ) -> Result<T, ActorError> {
        let (reply_tx,  reply_rx) = oneshot::channel();
        let msg = make_msg(reply_tx);
        self.tx.send(msg).await.map_err(|_| ActorError::SendFailed)?;
        reply_rx.await.map_err(|_| ActorError::ResponseDropped)
    } 

    pub async fn add(&self, delta: i64) -> Result<i64, ActorError> {
        self.request(|reply| Message::Add { delta, reply }).await
    }

    pub async fn get(&self) -> Result<i64, ActorError> {
        self.request(|reply| Message::Get { reply }).await
    }

    pub async fn stop(&self) -> Result<(), ActorError> {
        self.request(|reply| Message::Stop { reply }).await
    }
}

pub fn spawn_actor<State, Msg, Handler, Fut>(
    capacity: usize,
    initial_state: State,
    handler: Handler
) -> (Sender<Msg>, JoinHandle<ExitReason>)
where
    State: Send + 'static,
    Msg: Send + 'static,
    Handler: FnMut(&mut State, Msg) -> Fut + Send + 'static,
    Fut: Future<Output = ActorCtrl> + Send + 'static
{
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
    (tx, join)
}


pub fn spawn_counter(capacity: usize) -> (CounterHandle, JoinHandle<ExitReason>) {
    let (tx, join) = spawn_actor(
        capacity,
        0_i64,
        |state, msg| {
            let ctrl = match msg {
                Message::Add { delta, reply } => {
                    *state += delta;
                    let _ = reply.send(*state);
                    ActorCtrl::Continue
                }
                Message::Get { reply } => {
                    let _ = reply.send(*state);
                    ActorCtrl::Continue
                }
                Message::Stop { reply } => {
                    let _ = reply.send(());
                    ActorCtrl::Stop
                }
            };

            async move { ctrl }
        },
    );
    (CounterHandle { tx }, join)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_counter_add_get_happy_path() {
        let (handle, _join) = spawn_counter(8);

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
        let (handle, join) = spawn_counter(8);
        
        let stop_res = handle.stop().await;
        assert!(stop_res.is_ok());

        let join_res = join.await;
        assert!(join_res.is_ok());
    }

    #[tokio::test]
    async fn test_counter_handle_clone_works() {
        let (handle, _join) = spawn_counter(8);
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
        let (handle, join) = spawn_counter(8);
        let _ = handle.stop().await;
        let exit = join.await.expect("Actor task panicked");
        assert_eq!(exit, ExitReason::StoppedByMessage);
    }

    #[tokio::test]
    async fn test_exits_with_all_senders_dropped() {
        let (handle, join) = spawn_counter(8);
        let handle2 = handle.clone();

        drop(handle);
        drop(handle2);

        let exit = join.await.expect("actor task panicked");
        assert_eq!(exit, ExitReason::AllSendersDropped); 
    }
}

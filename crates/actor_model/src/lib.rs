use tokio::{
    sync::{
        oneshot::Sender,
        mpsc::{Receiver, Sender as MpscSender}
    },
    task::JoinHandle
};

#[derive(Debug)]
pub enum ActorError {
    SendFailed,
    ResponseDropped,
}

enum Message {
    Add { delta: i64, reply: Sender },
    Get { reply: Sender<i64> },
    Stop,
}

pub struct CounterActor {
    count: i64,
    rx: Receiver,
}

impl CounterActor {
    async fn run(mut self) {
        while let Some(msg) = self.rx.recv() await {
            match msg {
                M
            }
        }
    }
}

#[derive(Clone)]
pub struct CounterHandle {
    tx: MpscSender<Message>, 
}

pub fn spawn_counter(counter: usize) -> (CounterHandle, JoinHandle<()>) {
    let (tx, rx) = oneshot::channel();
}


// use tokio::sync::mpsc;
//
//   struct CounterActor {
//       count: i64,
//       rx: mpsc::Receiver<Msg>,
//   }
//
//   impl CounterActor {
//       async fn run(mut self) {
//           while let Some(msg) = self.rx.recv().await {
//               match msg {
//                   Msg::Add { delta, reply } => {
//                       self.count += delta;
//                       let _ = reply.send(self.count);
//                   }
//                   Msg::Get { reply } => {
//                       let _ = reply.send(self.count);
//                   }
//                   Msg::Stop { reply } => {
//                       let _ = reply.send(());
//                       break;
//                   }
//               }
//           }
//       }
//   }
//
//   And the spawn flow (still just a sketch):
//
//   pub fn spawn_counter(capacity: usize) -> (CounterHandle, JoinHandle<()>) {
//       let (tx, rx) = mpsc::channel::<Msg>(capacity);
//       let actor = CounterActor { count: 0, rx };
//
//       let join = tokio::spawn(actor.run());
//       let handle = CounterHandle { tx };
//
//       (handle, join)
//   }

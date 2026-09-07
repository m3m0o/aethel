use tokio::sync::mpsc;

pub struct WorkQueue<T> {
    sender: mpsc::Sender<T>,
    receiver: mpsc::Receiver<T>,
}
impl<T> WorkQueue<T> {
    pub fn bounded(capacity: usize) -> Result<Self, String> {
        if capacity == 0 {
            return Err("worker queue capacity must be greater than zero".into());
        }
        let (sender, receiver) = mpsc::channel(capacity);
        Ok(Self { sender, receiver })
    }
    pub fn sender(&self) -> mpsc::Sender<T> {
        self.sender.clone()
    }
    pub async fn receive(&mut self) -> Option<T> {
        self.receiver.recv().await
    }
}
#[cfg(test)]
mod tests {
    use super::WorkQueue;
    #[test]
    fn bounds_and_delivers_work() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut queue = WorkQueue::bounded(1).unwrap();
            let sender = queue.sender();
            sender.send(7).await.unwrap();
            assert_eq!(queue.receive().await, Some(7));
            assert!(WorkQueue::<u8>::bounded(0).is_err());
        });
    }
}

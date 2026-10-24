use serde_json::json;
use syneroym_core::mailbox::PullSummary;

use super::*;

#[derive(Debug)]
struct FakePuller(PullSummary);

#[async_trait::async_trait]
impl MailboxPullControl for FakePuller {
    async fn pull_now(&self) -> PullSummary {
        self.0
    }

    fn wake(&self) {}

    async fn release(&self, _service_id: &str, _mailbox: &str) {}
}

#[tokio::test]
async fn an_unwired_receiver_answers_method_not_found() {
    let err = pull_now(None, json!([])).await.unwrap_err();
    assert!(matches!(err, RpcError::MethodNotFound(msg) if msg.contains("no mailbox receiver")));
}

#[tokio::test]
async fn a_receiver_that_is_gone_answers_method_not_found() {
    let puller: Arc<dyn MailboxPullControl> = Arc::new(FakePuller(PullSummary::default()));
    let weak = Arc::downgrade(&puller);
    drop(puller);
    let err = pull_now(Some(&weak), json!(null)).await.unwrap_err();
    assert!(matches!(err, RpcError::MethodNotFound(_)));
}

#[tokio::test]
async fn the_summary_of_one_pass_is_the_answer() {
    let summary = PullSummary {
        services: 2,
        blocked: 1,
        registered: 1,
        delivered: 3,
        rejected: 1,
        not_acked: 0,
    };
    let puller: Arc<dyn MailboxPullControl> = Arc::new(FakePuller(summary));
    let weak = Arc::downgrade(&puller);
    for params in [json!([]), json!(null)] {
        let response = pull_now(Some(&weak), params).await.unwrap();
        assert_eq!(serde_json::from_value::<PullSummary>(response.payload).unwrap(), summary);
    }
}

#[tokio::test]
async fn parameters_are_refused() {
    let puller: Arc<dyn MailboxPullControl> = Arc::new(FakePuller(PullSummary::default()));
    let weak = Arc::downgrade(&puller);
    let err = pull_now(Some(&weak), json!(["x"])).await.unwrap_err();
    assert!(matches!(err, RpcError::InvalidParams(_)));
}

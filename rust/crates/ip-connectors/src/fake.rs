//! 测试用的假渠道（`docs/ip/m2-spec.md` D-09），用法和 `ip-models` 的脚本模型一样。
//!
//! ```ignore
//! let (channel, handle) = FakeChannel::new("fake");
//! // 把 channel 接到 Hub 和投递器上之后：
//! handle.say("会话-1", "u1", "点评一下本月 CPI").await?;   // may_start = true
//! handle.reply("会话-1", "u1", "再看看 PPI").await?;        // may_start = false
//! handle.click(&approval_id, 0, "u1").await?;
//! let sent = handle.next(Duration::from_secs(5)).await.expect("应当发出一条");
//! handle.fail_next(2, DeliverError::Retry { after: None, reason: "限流".into() });
//! ```
//!
//! 用户 `u1` 的显示名是 `用户u1`。每次 `say` / `reply` 的去重 key 都不同，新建的假渠道也不会
//! 和之前的重号；要模拟外部系统重复推送，把 `say` 返回的 [`Inbound`] 原样交给 [`FakeHandle::send`]。

pub mod slack;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ip_core::ApprovalId;
use tokio::sync::{mpsc, oneshot};

use crate::{
    BoxFuture, CancellationToken, CardClick, Channel, ConversationKey, DeliverError, Delivered, Hub, HubError, Inbound,
    Outbound, Sender,
};

/// 等 Hub 处理一条消息的最长时间；超过说明渠道没在运行。
const HUB_TIMEOUT: Duration = Duration::from_secs(10);

/// 去重 key 的编号在整个进程里递增：模拟重启时新建的假渠道不会和之前的撞号，
/// 和真实渠道一样（Slack 的 key 来自消息时间戳，不会重复）。
static NEXT_KEY: AtomicUsize = AtomicUsize::new(1);

fn next_key() -> usize {
    NEXT_KEY.fetch_add(1, Ordering::Relaxed)
}

enum Request {
    Message(Inbound, oneshot::Sender<Result<(), HubError>>),
    Click(CardClick, oneshot::Sender<Result<(), HubError>>),
}

/// 发出去的一样东西和假渠道给它的引用（`fake-1`、`fake-2`……）。
#[derive(Clone, Debug, PartialEq)]
pub struct Sent {
    pub out: Outbound,
    pub external_ref: String,
}

pub struct FakeChannel {
    name: String,
    requests: Mutex<Option<mpsc::Receiver<Request>>>,
    sent: mpsc::UnboundedSender<Sent>,
    failures: Mutex<VecDeque<DeliverError>>,
    attempts: AtomicUsize,
    refs: AtomicUsize,
}

/// 测试一侧：往渠道里发消息、点卡片，看渠道发出了什么。
pub struct FakeHandle {
    name: String,
    channel: Arc<FakeChannel>,
    requests: mpsc::Sender<Request>,
    sent: tokio::sync::Mutex<mpsc::UnboundedReceiver<Sent>>,
}

impl FakeChannel {
    pub fn new(name: &str) -> (Arc<Self>, FakeHandle) {
        let (req_tx, req_rx) = mpsc::channel(64);
        let (sent_tx, sent_rx) = mpsc::unbounded_channel();
        let channel = Arc::new(Self {
            name: name.to_string(),
            requests: Mutex::new(Some(req_rx)),
            sent: sent_tx,
            failures: Mutex::default(),
            attempts: AtomicUsize::new(0),
            refs: AtomicUsize::new(0),
        });
        let handle = FakeHandle {
            name: name.to_string(),
            channel: channel.clone(),
            requests: req_tx,
            sent: tokio::sync::Mutex::new(sent_rx),
        };
        (channel, handle)
    }
}

impl Channel for FakeChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(self: Arc<Self>, hub: Arc<dyn Hub>, shutdown: CancellationToken) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let Some(mut requests) = self.requests.lock().unwrap_or_else(|p| p.into_inner()).take() else {
                // 一个假渠道只能运行一次；要模拟重启就新建一个。
                return;
            };
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    request = requests.recv() => match request {
                        Some(Request::Message(inbound, reply)) => {
                            let _ = reply.send(hub.message(inbound).await);
                        }
                        Some(Request::Click(click, reply)) => {
                            let _ = reply.send(hub.click(click).await);
                        }
                        None => break,
                    },
                }
            }
        })
    }

    fn deliver<'a>(&'a self, out: &'a Outbound) -> BoxFuture<'a, Result<Delivered, DeliverError>> {
        Box::pin(async move {
            self.attempts.fetch_add(1, Ordering::Relaxed);
            if let Some(error) = self.failures.lock().unwrap_or_else(|p| p.into_inner()).pop_front() {
                return Err(error);
            }
            let external_ref = format!("fake-{}", self.refs.fetch_add(1, Ordering::Relaxed) + 1);
            let _ = self.sent.send(Sent {
                out: out.clone(),
                external_ref: external_ref.clone(),
            });
            Ok(Delivered { external_ref })
        })
    }
}

impl FakeHandle {
    fn sender(user: &str) -> Sender {
        Sender {
            external_id: user.to_string(),
            display_name: format!("用户{user}"),
        }
    }

    fn inbound(&self, conversation: &str, user: &str, text: &str, may_start: bool) -> Inbound {
        Inbound {
            channel: self.name.clone(),
            dedupe_key: format!("{}:{}", self.name, next_key()),
            conversation: conversation.into(),
            sender: Self::sender(user),
            text: text.to_string(),
            may_start,
        }
    }

    /// 发一条能开启新会话的消息（比如 Slack 里 @机器人），等 Hub 处理完。返回发出的消息，可用来重推。
    pub async fn say(&self, conversation: &str, user: &str, text: &str) -> Result<Inbound, HubError> {
        let inbound = self.inbound(conversation, user, text, true);
        self.send(inbound.clone()).await.map(|()| inbound)
    }

    /// 在会话里接着说话（不能开启新会话），等 Hub 处理完。
    pub async fn reply(&self, conversation: &str, user: &str, text: &str) -> Result<Inbound, HubError> {
        let inbound = self.inbound(conversation, user, text, false);
        self.send(inbound.clone()).await.map(|()| inbound)
    }

    /// 原样交给 Hub，比如重推一条已经发过的消息。
    pub async fn send(&self, inbound: Inbound) -> Result<(), HubError> {
        let (reply, answer) = oneshot::channel();
        self.request(Request::Message(inbound, reply), answer).await
    }

    /// 点卡片上的第 `choice` 个选项，等 Hub 处理完。
    pub async fn click(&self, approval: &ApprovalId, choice: u32, user: &str) -> Result<(), HubError> {
        let click = CardClick {
            channel: self.name.clone(),
            dedupe_key: format!("{}:click:{}", self.name, next_key()),
            approval_id: approval.clone(),
            choice,
            sender: Self::sender(user),
        };
        let (reply, answer) = oneshot::channel();
        self.request(Request::Click(click, reply), answer).await
    }

    async fn request(&self, request: Request, answer: oneshot::Receiver<Result<(), HubError>>) -> Result<(), HubError> {
        let stopped = || HubError::Failed("假渠道没有在运行".into());
        self.requests.send(request).await.map_err(|_| stopped())?;
        match tokio::time::timeout(HUB_TIMEOUT, answer).await {
            Ok(Ok(result)) => result,
            _ => Err(stopped()),
        }
    }

    /// 等下一样发出去的东西；`within` 内没有就返回 `None`。
    pub async fn next(&self, within: Duration) -> Option<Sent> {
        let mut sent = self.sent.lock().await;
        tokio::time::timeout(within, sent.recv()).await.ok().flatten()
    }

    /// 等 `count` 样发出去的东西，按发出的顺序返回；`within` 内凑不齐时返回已有的。
    pub async fn take(&self, count: usize, within: Duration) -> Vec<Sent> {
        let deadline = tokio::time::Instant::now() + within;
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match self.next(left).await {
                Some(s) => out.push(s),
                None => break,
            }
        }
        out
    }

    /// 让接下来 `n` 次发送都返回 `error`。
    pub fn fail_next(&self, n: usize, error: DeliverError) {
        let mut failures = self.channel.failures.lock().unwrap_or_else(|p| p.into_inner());
        failures.extend(std::iter::repeat_n(error, n));
    }

    /// 到目前为止调用 `deliver` 的次数，含失败的。
    pub fn attempts(&self) -> usize {
        self.channel.attempts.load(Ordering::Relaxed)
    }

    /// 渠道里这个会话的 key。
    pub fn conversation(key: &str) -> ConversationKey {
        key.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 记下收到的消息和点击；`u9` 的点击按规则拒绝。
    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<String>>,
    }

    impl Hub for Recorder {
        fn message(&self, inbound: Inbound) -> BoxFuture<'_, Result<(), HubError>> {
            Box::pin(async move {
                let line = format!(
                    "{}|{}|{}|{}",
                    inbound.conversation, inbound.sender.display_name, inbound.text, inbound.may_start
                );
                self.seen.lock().unwrap().push(line);
                Ok(())
            })
        }

        fn click(&self, click: CardClick) -> BoxFuture<'_, Result<(), HubError>> {
            Box::pin(async move {
                if click.sender.external_id == "u9" {
                    return Err(HubError::Rejected("只有提需求的人能选".into()));
                }
                self.seen
                    .lock()
                    .unwrap()
                    .push(format!("click|{}|{}", click.approval_id, click.choice));
                Ok(())
            })
        }

        // 假渠道直接调 message 和 click，不存信封。
        fn stash(&self, _id: &str, _payload: &str) -> BoxFuture<'_, Result<bool, HubError>> {
            Box::pin(async { Ok(true) })
        }

        fn stashed(&self) -> BoxFuture<'_, Result<Vec<crate::Stashed>, HubError>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn settle(&self, _id: &str, _settle: crate::Settle) -> BoxFuture<'_, Result<(), HubError>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn fake_channel_round_trip() {
        let (channel, handle) = FakeChannel::new("fake");
        let hub = Arc::new(Recorder::default());
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(channel.clone().run(hub.clone(), shutdown.clone()));

        let first = handle.say("c1", "u1", "你好").await.unwrap();
        let again = handle.reply("c1", "u1", "还在吗").await.unwrap();
        assert_ne!(first.dedupe_key, again.dedupe_key);
        handle.send(first.clone()).await.unwrap();
        handle.click(&"a1".into(), 2, "u1").await.unwrap();
        assert_eq!(
            handle.click(&"a1".into(), 0, "u9").await,
            Err(HubError::Rejected("只有提需求的人能选".into()))
        );
        assert_eq!(
            *hub.seen.lock().unwrap(),
            [
                "c1|用户u1|你好|true",
                "c1|用户u1|还在吗|false",
                "c1|用户u1|你好|true",
                "click|a1|2"
            ]
        );

        let text = Outbound::Text {
            conversation: "c1".into(),
            text: "一".into(),
        };
        handle.fail_next(1, DeliverError::Permanent("没有这个会话".into()));
        assert_eq!(
            channel.deliver(&text).await,
            Err(DeliverError::Permanent("没有这个会话".into()))
        );
        assert_eq!(channel.deliver(&text).await.unwrap().external_ref, "fake-1");
        assert_eq!(channel.deliver(&text).await.unwrap().external_ref, "fake-2");
        assert_eq!(handle.attempts(), 3);
        let sent = handle.take(2, Duration::from_secs(1)).await;
        assert_eq!(
            sent.iter().map(|s| s.external_ref.as_str()).collect::<Vec<_>>(),
            ["fake-1", "fake-2"]
        );
        assert_eq!(sent[0].out, text);
        assert!(handle.next(Duration::from_millis(50)).await.is_none());

        shutdown.cancel();
        task.await.unwrap();
        assert_eq!(
            handle.say("c1", "u1", "停了吗").await,
            Err(HubError::Failed("假渠道没有在运行".into()))
        );
    }
}

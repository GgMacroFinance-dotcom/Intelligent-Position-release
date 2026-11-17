//! 事件序号和重连补发（`docs/ip/m2-spec.md` D-29，backlog F-6）。
//!
//! 一个转发任务从运行时收事件，编好号、序列化成事件帧，留最近一段，再广播给各个 `/ws` 连接。
//! 各连接直接发同一份文字，不再每个连接各序列化一遍。重连时客户端带上次收到的序号，
//! 留着的事件里有就补发，没有就让客户端重新拉。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::ws::Utf8Bytes;
use ip_runtime::RuntimeEvent;
use tokio::sync::broadcast;

use crate::protocol::ServerFrame;

/// `/ws` 协议版本号，只有不兼容的改动才加一。
pub const PROTOCOL_VERSION: u32 = 1;
/// 最多留多少个事件。
pub const KEEP_EVENTS: usize = 4096;
/// 留着的事件帧合计最多多少字节。
pub const KEEP_BYTES: usize = 4 << 20;
/// 发给各连接的广播通道能积压多少帧；跟不上时从留着的事件里补。
pub const CHANNEL_CAPACITY: usize = 1024;

/// 编好号、序列化好的一帧事件。`frame` 是引用计数的，发给多个连接不复制。
#[derive(Clone, Debug)]
pub struct Stamped {
    pub seq: u64,
    pub frame: Utf8Bytes,
}

/// 补不上：`since` 之后有 `skipped` 个事件已经不在了（不知道时为 0）。
#[derive(Debug, PartialEq, Eq)]
pub struct Gap {
    pub skipped: u64,
}

struct Ring {
    /// 下一个事件的序号。
    next: u64,
    /// 按序号排好、连续的事件。
    buf: VecDeque<Stamped>,
    bytes: usize,
}

pub struct EventLog {
    epoch: String,
    keep_events: usize,
    keep_bytes: usize,
    ring: Mutex<Ring>,
    tx: broadcast::Sender<Stamped>,
    pumping: AtomicBool,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new(KEEP_EVENTS, KEEP_BYTES, CHANNEL_CAPACITY)
    }
}

impl EventLog {
    /// 最多留 `keep_events` 个、合计 `keep_bytes` 字节的事件；广播通道积压 `channel_capacity` 帧。
    pub fn new(keep_events: usize, keep_bytes: usize, channel_capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(channel_capacity.max(1));
        Self {
            epoch: new_epoch(),
            keep_events: keep_events.max(1),
            keep_bytes,
            ring: Mutex::new(Ring {
                next: 1,
                buf: VecDeque::new(),
                bytes: 0,
            }),
            tx,
            pumping: AtomicBool::new(false),
        }
    }

    /// 本次启动的标识。
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    /// 最新的序号；还没有事件时是 0。
    pub fn latest(&self) -> u64 {
        self.lock().next - 1
    }

    /// 收听新事件。要先收听再调 [`EventLog::since`]，才不会漏掉两步之间的事件。
    pub fn subscribe(&self) -> broadcast::Receiver<Stamped> {
        self.tx.subscribe()
    }

    /// 编号、序列化、留下，再广播出去。只由转发任务调用，所以广播的顺序就是序号的顺序。
    pub fn push(&self, event: RuntimeEvent) -> u64 {
        let mut ring = self.lock();
        let seq = ring.next;
        ring.next += 1;
        let frame = Utf8Bytes::from(
            ServerFrame::Event {
                seq,
                event: Box::new(event),
            }
            .to_json(),
        );
        ring.bytes += frame.len();
        ring.buf.push_back(Stamped {
            seq,
            frame: frame.clone(),
        });
        while ring.buf.len() > 1 && (ring.buf.len() > self.keep_events || ring.bytes > self.keep_bytes) {
            if let Some(old) = ring.buf.pop_front() {
                ring.bytes -= old.frame.len();
            }
        }
        // 没有连接时发送失败，不要紧。
        let _ = self.tx.send(Stamped { seq, frame });
        seq
    }

    /// 转发任务自己跟不上、丢了 `skipped` 个事件：序号跳过去，留着的事件清空，
    /// 这样留着的总是连续的，要补这一段的连接都会收到 `lagged`。
    pub fn skip(&self, skipped: u64) {
        let mut ring = self.lock();
        ring.next += skipped;
        ring.buf.clear();
        ring.bytes = 0;
    }

    /// `since` 之后的所有事件。中间有不在了的就返回 [`Gap`]。
    pub fn since(&self, since: u64) -> Result<Vec<Stamped>, Gap> {
        let ring = self.lock();
        let latest = ring.next - 1;
        if since > latest {
            // 客户端的序号比服务端还新，只能是别的启动留下的。
            return Err(Gap { skipped: 0 });
        }
        if since == latest {
            return Ok(Vec::new());
        }
        let oldest = ring.buf.front().map_or(ring.next, |s| s.seq);
        if since + 1 < oldest {
            return Err(Gap {
                skipped: oldest - since - 1,
            });
        }
        let start = usize::try_from(since + 1 - oldest).unwrap_or(usize::MAX);
        Ok(ring.buf.iter().skip(start).cloned().collect())
    }

    /// 第一次调用时启动转发任务：从运行时收事件，交给 [`EventLog::push`]。之后再调用什么都不做。
    pub fn start_pump(self: &Arc<Self>, mut events: broadcast::Receiver<RuntimeEvent>) {
        if self.pumping.swap(true, Ordering::SeqCst) {
            return;
        }
        let log = self.clone();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        log.push(event);
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "事件转发跟不上运行时，跳过了一些事件");
                        log.skip(n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.ring.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// 本次启动的标识：启动时间、进程号，再加一个进程内计数（测试里一个进程会建好几个）。
fn new_epoch() -> String {
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{:x}-{n:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(n: u64) -> RuntimeEvent {
        RuntimeEvent::Delta {
            agent_id: "a".into(),
            thread_id: "t".into(),
            text: format!("第 {n} 段"),
        }
    }

    #[test]
    fn numbers_and_replays_events() {
        let log = EventLog::new(10, usize::MAX, 16);
        assert_eq!(log.latest(), 0);
        assert!(log.since(0).unwrap().is_empty());
        for n in 1..=5 {
            assert_eq!(log.push(event(n)), n);
        }
        assert_eq!(log.latest(), 5);
        let seqs: Vec<u64> = log.since(2).unwrap().iter().map(|s| s.seq).collect();
        assert_eq!(seqs, [3, 4, 5]);
        assert!(log.since(5).unwrap().is_empty());
        let frame: serde_json::Value = serde_json::from_str(log.since(4).unwrap()[0].frame.as_str()).unwrap();
        assert_eq!(frame["type"], "event");
        assert_eq!(frame["seq"], 5);
        assert_eq!(frame["event"]["text"], "第 5 段");
    }

    #[test]
    fn reports_a_gap_once_events_are_dropped() {
        let log = EventLog::new(3, usize::MAX, 16);
        for n in 1..=10 {
            log.push(event(n));
        }
        // 只留 8、9、10
        assert_eq!(log.since(7).unwrap().len(), 3);
        assert_eq!(log.since(5).unwrap_err(), Gap { skipped: 2 });
        // 比最新的还新：别的启动留下的序号
        assert_eq!(log.since(11).unwrap_err(), Gap { skipped: 0 });
    }

    #[test]
    fn keeps_within_the_byte_budget() {
        let one = ServerFrame::Event {
            seq: 1,
            event: Box::new(event(1)),
        }
        .to_json()
        .len();
        let log = EventLog::new(1000, one * 2 + 1, 16);
        for n in 1..=5 {
            log.push(event(n));
        }
        assert_eq!(log.since(3).unwrap().len(), 2);
        assert!(log.since(2).is_err());
        // 单个事件超过上限也至少留最新的一个
        let tiny = EventLog::new(1000, 1, 16);
        tiny.push(event(1));
        assert_eq!(tiny.since(0).unwrap().len(), 1);
    }

    #[test]
    fn skipping_clears_the_buffer_so_replays_stay_contiguous() {
        let log = EventLog::new(100, usize::MAX, 16);
        for n in 1..=3 {
            log.push(event(n));
        }
        log.skip(4); // 4 到 7 丢了
        assert_eq!(log.push(event(8)), 8);
        assert_eq!(log.since(3).unwrap_err(), Gap { skipped: 4 });
        assert_eq!(log.since(7).unwrap().len(), 1);
    }

    #[test]
    fn epochs_differ_between_logs() {
        assert_ne!(EventLog::default().epoch(), EventLog::default().epoch());
    }
}

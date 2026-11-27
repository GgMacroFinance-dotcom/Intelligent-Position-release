//! 星图上人固定的轨道（`docs/ip/m2-spec.md` D-40，桌面外壳后端适配清单第 1 项）。
//!
//! App 宽窗口的天穹有三条固定轨道（0 内环、1 中环、2 外环），人能把员工拖到某条轨道上固定住；
//! 满环时 App 按确定的规则把别人挤到外侧，一次拖放改动的几位员工一起提交。
//! 主机只存结果、校验、广播给其他设备，不重算挤人：规则在 App 里，各端算出来一样。
//!
//! 整张表存在设置表的一个键里，读改写由一把锁串行化。员工从 ip.yaml 里删掉以后，
//! 他的记录留在库里但不再列出；同一个 id 加回来时还在原来的轨道上。

use std::collections::BTreeMap;

use ip_core::{AgentId, ORBIT_TRACKS, OrbitPin, now_ms};
use ip_runtime::RuntimeEvent;

use crate::App;
use crate::protocol::OrbitPinChange;

const PINS_KEY: &str = "star_map.orbit_pins";
/// 一次最多改几位员工。一次拖放最多改动“拖进去的 + 每条轨道挤出一位”，远小于它。
pub const MAX_CHANGES: usize = 256;

/// 现在固定了轨道的员工，按员工 id 排；已经不在组织里的员工不列。
pub async fn list(app: &App) -> Result<Vec<OrbitPin>, String> {
    let saved = load(app).await?;
    Ok(visible(app, saved))
}

/// 校验并整批应用一次拖放的改动，存下来，推 `orbit_pins_changed`。回复改完后的表。
pub async fn set(app: &App, changes: Vec<OrbitPinChange>) -> Result<Vec<OrbitPin>, String> {
    if changes.len() > MAX_CHANGES {
        return Err(format!("一次最多改 {MAX_CHANGES} 位员工的轨道"));
    }
    let agents: std::collections::HashSet<AgentId> = app.runtime.agents().into_iter().map(|a| a.id).collect();
    let now = now_ms();
    for change in &changes {
        if !agents.contains(&change.agent_id) {
            return Err(format!("找不到员工 {}", change.agent_id));
        }
        if let Some(track) = change.orbit_track
            && track >= ORBIT_TRACKS
        {
            return Err(format!(
                "轨道只有 0（内环）、1（中环）、2（外环），没有 {track}（员工 {}）",
                change.agent_id
            ));
        }
        if change.orbit_pinned_at.is_some_and(|at| at < 0) {
            return Err(format!("员工 {} 的 orbit_pinned_at 不能是负数", change.agent_id));
        }
    }
    let _guard = app.star_map_lock.lock().await;
    let mut pins = load(app).await?;
    let before = pins.clone();
    // 同一位员工出现几次，以最后一次为准。
    for change in changes {
        match change.orbit_track {
            Some(track) => {
                pins.insert(
                    change.agent_id.clone(),
                    OrbitPin {
                        agent_id: change.agent_id,
                        orbit_track: track,
                        orbit_pinned_at: change.orbit_pinned_at.unwrap_or(now),
                    },
                );
            }
            None => {
                pins.remove(&change.agent_id);
            }
        }
    }
    if pins != before {
        let value = serde_json::to_value(pins.values().collect::<Vec<_>>()).map_err(|e| e.to_string())?;
        app.runtime
            .store()
            .put_setting(PINS_KEY, value)
            .await
            .map_err(|e| e.to_string())?;
    }
    let shown = visible(app, pins);
    if shown != visible(app, before) {
        app.runtime
            .publish(RuntimeEvent::OrbitPinsChanged { pins: shown.clone() });
    }
    Ok(shown)
}

async fn load(app: &App) -> Result<BTreeMap<AgentId, OrbitPin>, String> {
    let saved = app.runtime.store().setting(PINS_KEY).await.map_err(|e| e.to_string())?;
    let Some(value) = saved else {
        return Ok(BTreeMap::new());
    };
    let pins: Vec<OrbitPin> = serde_json::from_value(value).map_err(|e| format!("存下的星图轨道读不出来：{e}"))?;
    Ok(pins.into_iter().map(|p| (p.agent_id.clone(), p)).collect())
}

/// 只留现在还在组织里的员工。
fn visible(app: &App, pins: BTreeMap<AgentId, OrbitPin>) -> Vec<OrbitPin> {
    let agents: std::collections::HashSet<AgentId> = app.runtime.agents().into_iter().map(|a| a.id).collect();
    pins.into_values().filter(|p| agents.contains(&p.agent_id)).collect()
}

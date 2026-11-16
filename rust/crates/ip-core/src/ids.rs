//! 各类 ID。新建的 ID 用 UUIDv7，按时间有序，作数据库主键时插入局部性好。

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        // 单字段元组结构，serde 默认就序列化成里面的字符串。
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
        #[ts(export)]
        pub struct $name(String);

        impl $name {
            /// 新建一个按时间有序的 ID。
            pub fn new() -> Self {
                Self(uuid::Uuid::now_v7().to_string())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// 岗位 ID，通常是人写的短名，如 `finance-ap`。
    RoleId
);
id_type!(
    /// 数字员工 ID。
    AgentId
);
id_type!(
    /// 对话 ID。
    ThreadId
);
id_type!(
    /// 消息 ID。
    MessageId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ids_are_time_ordered() {
        let first = MessageId::new();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = MessageId::new();
        assert!(first < second);
    }
}
id_type!(
    /// 任务 ID。
    TaskId
);
id_type!(
    /// 审批（决策卡片）ID。
    ApprovalId
);
